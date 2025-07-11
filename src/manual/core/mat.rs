use std::convert::TryInto;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ops::Deref;
use std::{fmt, mem, ptr, slice};

pub use mat_::*;

use crate::boxed_ref::{BoxedRef, BoxedRefMut};
use crate::core::{MatConstIterator, MatExpr, MatSize, Point, Rect, Scalar, Size, UMat};
use crate::manual::core::DataType;
use crate::prelude::*;
use crate::{core, input_output_array, input_output_array_vector, Error, Result};

mod mat_;

#[inline(always)]
/// We rely on OpenCV to make sure that the pointer is correctly aligned
unsafe fn convert_ptr<'r, T>(r: *const u8) -> &'r T {
	unsafe { &*(r.cast::<T>()) }
}

#[inline(always)]
/// We rely on OpenCV to make sure that the pointer is correctly aligned
unsafe fn convert_ptr_mut<'r, T>(r: *mut u8) -> &'r mut T {
	unsafe { &mut *(r.cast::<T>()) }
}

#[inline]
fn match_format<T: DataType>(mat_type: i32) -> Result<()> {
	let out_type = T::opencv_type();
	if mat_type == out_type {
		Ok(())
	} else {
		let mat_type = core::type_to_string(mat_type)?;
		let out_type = core::type_to_string(out_type)?;
		Err(Error::new(
			core::StsUnmatchedFormats,
			format!("Mat type is: {mat_type}, but requested type is: {out_type}"),
		))
	}
}

fn match_indices(mat: &(impl MatTraitConst + ?Sized), idx: &[i32]) -> Result<()> {
	let mat_size = mat.mat_size();
	let size = &*mat_size;
	if size.len() != idx.len() {
		return Err(Error::new(
			core::StsUnmatchedSizes,
			format!(
				"Amount of Mat dimensions: {} doesn't match the amount of requested indices: {}",
				size.len(),
				idx.len()
			),
		));
	}
	if let Some((out_idx, (out_idx_val, out_size))) = idx
		.iter()
		.zip(size)
		.enumerate()
		.find(|(_, (idx_val, &size))| !(0..size).contains(idx_val))
	{
		Err(Error::new(
			core::StsOutOfRange,
			format!("Index: {out_idx_val} along dimension: {out_idx} out of bounds 0..{out_size}"),
		))
	} else {
		Ok(())
	}
}

#[inline]
fn match_total(mat: &(impl MatTraitConst + ?Sized), idx: i32) -> Result<()> {
	let size = mat.total();
	// safe because of the `0 <= idx` check
	if 0 <= idx && (idx as usize) < size {
		Ok(())
	} else {
		Err(Error::new(
			core::StsOutOfRange,
			format!("Index: {idx} out of bounds: 0..{size}"),
		))
	}
}

#[inline]
fn match_is_continuous(mat: &(impl MatTraitConst + ?Sized)) -> Result<()> {
	if mat.is_continuous() {
		Ok(())
	} else {
		Err(Error::new(
			core::StsUnmatchedSizes,
			"Mat is not continuous, operation is not applicable",
		))
	}
}

fn match_length(sizes: &[i32], slice_len: usize, size_mul: usize) -> Result<()> {
	let mut expected_len: u64 = 1;
	for (i, size) in sizes.iter().enumerate() {
		let size =
			u64::try_from(*size).map_err(|_| Error::new(core::StsOutOfRange, format!("Dimension {i} must not be negative")))?;
		expected_len = expected_len.saturating_mul(size);
	}
	if size_mul > 1 {
		expected_len = expected_len.saturating_mul(
			u64::try_from(size_mul).map_err(|_| Error::new(core::StsOutOfRange, "Size multiplier must fit in u64"))?,
		);
	}
	let slice_len = u64::try_from(slice_len).map_err(|_| Error::new(core::StsOutOfRange, "Length must fit in u64"))?;
	if expected_len != slice_len {
		let msg = match sizes {
			[rows, cols] => {
				format!("The length of the slice: {slice_len} must be: {expected_len} to match the passed row: {rows} and column: {cols} counts")
			}
			_ => {
				format!("The length of the slice: {slice_len} must be: {expected_len} to match the passed dimensions: {sizes:?}")
			}
		};
		return Err(Error::new(core::StsUnmatchedSizes, msg));
	}
	Ok(())
}

#[inline(always)]
fn idx_to_row_col(mat: &(impl MatTraitConst + ?Sized), i0: i32) -> Result<(i32, i32)> {
	Ok(if mat.is_continuous() {
		(0, i0)
	} else {
		let mat_size = mat.size()?;
		if mat_size.width == 1 {
			(0, i0)
		} else if mat_size.height == 1 {
			(i0, 0)
		} else {
			let i = i0 / mat_size.height;
			(i, i0 - i * mat_size.height)
		}
	})
}

#[inline]
fn row_count_i32(row_count: usize) -> Result<i32> {
	i32::try_from(row_count).map_err(|_| Error::new(core::StsBadArg, format!("Row count: {row_count} is too high")))
}

#[inline]
fn col_count_i32(col_count: usize) -> Result<i32> {
	i32::try_from(col_count).map_err(|_| Error::new(core::StsBadArg, format!("Column count: {col_count} is too high")))
}

impl Mat {
	/// Create new `Mat` from the iterator of known size
	pub fn from_exact_iter<T: DataType>(s: impl ExactSizeIterator<Item = T>) -> Result<Self> {
		let mut out = unsafe { Self::new_rows_cols(row_count_i32(s.len())?, 1, T::opencv_type()) }?;
		for (i, x) in s.enumerate() {
			// safe because `row_count_i32` ensures that len of `s` fits `i32`
			let i = i as i32;
			unsafe { ptr::write(out.at_unchecked_mut::<T>(i)?, x) };
		}
		Ok(out)
	}

	/// Create a new `Mat` from a single-dimensional slice
	#[inline]
	pub fn from_slice<T: DataType>(s: &[T]) -> Result<BoxedRef<Self>> {
		Self::new_rows_cols_with_data(1, i32::try_from(s.len())?, s)
	}

	/// Create a new `Mat` from a single-dimensional byte slice
	#[inline]
	pub fn from_bytes<T: DataType>(s: &[u8]) -> Result<BoxedRef<Self>> {
		let rem = s.len() % mem::size_of::<T>();
		if rem != 0 {
			return Err(Error::new(
				core::StsBadArg,
				format!(
					"Unexpected number of bytes: {} the indicated type, expected multiple of {}",
					s.len(),
					T::opencv_channels()
				),
			));
		}
		let len = s.len() / mem::size_of::<T>();
		Self::new_rows_cols_with_bytes::<T>(1, i32::try_from(len)?, s)
	}

	/// Create a new `Mat` from a mutable single-dimensional byte slice
	#[inline]
	pub fn from_bytes_mut<T: DataType>(s: &mut [u8]) -> Result<BoxedRefMut<Self>> {
		let rem = s.len() % mem::size_of::<T>();
		if rem != 0 {
			return Err(Error::new(
				core::StsBadArg,
				format!(
					"Unexpected number of bytes: {} the indicated type, expected multiple of {}",
					s.len(),
					T::opencv_channels()
				),
			));
		}
		let len = s.len() / mem::size_of::<T>();
		Self::new_rows_cols_with_bytes_mut::<T>(1, i32::try_from(len)?, s)
	}

	/// Create a new `Mat` from a mutable single-dimensional slice
	#[inline]
	pub fn from_slice_mut<T: DataType>(s: &mut [T]) -> Result<BoxedRefMut<Self>> {
		Self::new_rows_cols_with_data_mut(1, i32::try_from(s.len())?, s)
	}

	/// Create a new `Mat` by copying the data from a slice of slices
	///
	/// Every subslice must have the same length, otherwise an error is returned.
	pub fn from_slice_2d<T: DataType>(s: &[impl AsRef<[T]>]) -> Result<Self> {
		let col_count = if let Some(first_row) = s.first() {
			col_count_i32(first_row.as_ref().len())?
		} else {
			0
		};
		let row_count = if col_count > 0 {
			row_count_i32(s.len())?
		} else {
			0
		};
		let mut out = Self::new_rows_cols_with_default(row_count, col_count, T::opencv_type(), Scalar::all(0.))?;
		if row_count > 0 && col_count > 0 {
			for (row_n, row) in s.iter().enumerate() {
				// safe because `row_count_i32` ensures that len of `s` fits `i32`
				let row_n = row_n as i32;
				let trg = out.at_row_mut(row_n)?;
				let src = row.as_ref();
				if trg.len() != src.len() {
					return Err(Error::new(
						core::StsUnmatchedSizes,
						format!(
							"Unexpected number of items: {} in a row index: {row_n}, expected: {}",
							src.len(),
							trg.len(),
						),
					));
				}
				trg.copy_from_slice(src);
			}
		}
		Ok(out)
	}

	/// Create a new `Mat` that references a single-dimensional slice with custom shape
	#[inline]
	pub fn new_rows_cols_with_data<T: DataType>(rows: i32, cols: i32, data: &[T]) -> Result<BoxedRef<Self>> {
		match_length(&[rows, cols], data.len(), 1)?;
		let m = unsafe {
			Self::new_rows_cols_with_data_unsafe_def(rows, cols, T::opencv_type(), data.as_ptr().cast::<c_void>().cast_mut())
		}?;
		Ok(<BoxedRef<Mat>>::from(m))
	}

	/// Create a new `Mat` that references a single-dimensional byte slice with custom shape
	#[inline]
	pub fn new_rows_cols_with_bytes<T: DataType>(rows: i32, cols: i32, data: &[u8]) -> Result<BoxedRef<Self>> {
		match_length(&[rows, cols], data.len(), mem::size_of::<T>())?;
		let m = unsafe {
			Self::new_rows_cols_with_data_unsafe_def(rows, cols, T::opencv_type(), data.as_ptr().cast::<c_void>().cast_mut())
		}?;
		Ok(<BoxedRef<Mat>>::from(m))
	}

	/// Create a new mutable `Mat` that references a single-dimensional slice with custom shape
	#[inline]
	pub fn new_rows_cols_with_data_mut<T: DataType>(rows: i32, cols: i32, data: &mut [T]) -> Result<BoxedRefMut<Self>> {
		match_length(&[rows, cols], data.len(), 1)?;
		let m =
			unsafe { Self::new_rows_cols_with_data_unsafe_def(rows, cols, T::opencv_type(), data.as_mut_ptr().cast::<c_void>()) }?;
		Ok(<BoxedRefMut<Mat>>::from(m))
	}

	/// Create a new mutable `Mat` that references a single-dimensional byte slice with custom shape
	#[inline]
	pub fn new_rows_cols_with_bytes_mut<T: DataType>(rows: i32, cols: i32, data: &mut [u8]) -> Result<BoxedRefMut<Self>> {
		match_length(&[rows, cols], data.len(), mem::size_of::<T>())?;
		let m =
			unsafe { Self::new_rows_cols_with_data_unsafe_def(rows, cols, T::opencv_type(), data.as_mut_ptr().cast::<c_void>()) }?;
		Ok(<BoxedRefMut<Mat>>::from(m))
	}

	/// Create a new `Mat` that references a single-dimensional slice with custom shape
	#[inline]
	pub fn new_size_with_data<T: DataType>(size: Size, data: &[T]) -> Result<BoxedRef<Self>> {
		match_length(&[size.width, size.height], data.len(), 1)?;
		let m = unsafe { Self::new_size_with_data_unsafe_def(size, T::opencv_type(), data.as_ptr().cast::<c_void>().cast_mut()) }?;
		Ok(<BoxedRef<Mat>>::from(m))
	}

	/// Create a new mutable `Mat` that references a single-dimensional slice with custom shape
	#[inline]
	pub fn new_size_with_data_mut<T: DataType>(size: Size, data: &mut [T]) -> Result<BoxedRefMut<Self>> {
		match_length(&[size.width, size.height], data.len(), 1)?;
		let m = unsafe { Self::new_size_with_data_unsafe_def(size, T::opencv_type(), data.as_mut_ptr().cast::<c_void>()) }?;
		Ok(<BoxedRefMut<Mat>>::from(m))
	}

	/// Create a new `Mat` that references a single-dimensional slice with custom shape
	#[inline]
	pub fn new_nd_with_data<'data, T: DataType>(sizes: &[i32], data: &'data [T]) -> Result<BoxedRef<'data, Self>> {
		match_length(sizes, data.len(), 1)?;
		let m = unsafe { Self::new_nd_with_data_unsafe_def(sizes, T::opencv_type(), data.as_ptr().cast::<c_void>().cast_mut()) }?;
		Ok(<BoxedRef<Mat>>::from(m))
	}

	/// Create a new `Mat` that references a single-dimensional slice with custom shape
	#[inline]
	pub fn new_nd_with_data_mut<'data, T: DataType>(sizes: &[i32], data: &'data mut [T]) -> Result<BoxedRefMut<'data, Self>> {
		match_length(sizes, data.len(), 1)?;
		let m = unsafe { Self::new_nd_with_data_unsafe_def(sizes, T::opencv_type(), data.as_mut_ptr().cast::<c_void>()) }?;
		Ok(<BoxedRefMut<Mat>>::from(m))
	}

	/// Returns 2 mutable ROIs into a single `Mat` as long as they do not intersect
	pub fn roi_2_mut<MAT: MatTrait>(m: &mut MAT, roi1: Rect, roi2: Rect) -> Result<(BoxedRefMut<Mat>, BoxedRefMut<Mat>)> {
		if (roi1 & roi2).empty() {
			// safe because we made sure that the interest areas do not intersect
			let m2 = unsafe { (m as *mut MAT).as_mut().expect("Can't fail") };
			let out1 = Mat::roi_mut(m, roi1)?;
			let out2 = Mat::roi_mut(m2, roi2)?;
			Ok((out1, out2))
		} else {
			Err(Error::new(core::StsBadArg, "ROIs must not intersect"))
		}
	}
}

pub struct MatIter<'m, T> {
	iter: Option<MatConstIterator>,
	_d: PhantomData<&'m T>,
}

impl<T: DataType> MatIter<'_, T> {
	pub fn new(iter: MatConstIterator) -> Result<Self> {
		match_format::<T>(iter.typ())?;
		Ok(Self {
			iter: Some(iter),
			_d: PhantomData,
		})
	}
}

impl<T: DataType> Iterator for MatIter<'_, T> {
	type Item = (Point, T);

	fn next(&mut self) -> Option<Self::Item> {
		self.iter.as_mut().and_then(|iter| {
			if iter.has_elements() {
				// the type is checked by the `MatIter::new()` and we ensure there are still elements by calling `has_elements()`
				let cur = *unsafe { convert_ptr(iter.ptr()) };
				let pos = iter.pos().ok()?;
				iter.seek(1, true).ok()?;
				Some((pos, cur))
			} else {
				None
			}
		})
	}
}

pub struct MatIterMut<'m, T> {
	iter: Option<MatConstIterator>,
	_d: PhantomData<&'m mut T>,
}

impl<T: DataType> MatIterMut<'_, T> {
	pub fn new(iter: MatConstIterator) -> Result<Self> {
		match_format::<T>(iter.typ())?;
		Ok(Self {
			iter: Some(iter),
			_d: PhantomData,
		})
	}
}

impl<'m, T: DataType> Iterator for MatIterMut<'m, T> {
	type Item = (Point, &'m mut T);

	fn next(&mut self) -> Option<Self::Item> {
		self.iter.as_mut().and_then(|iter| {
			if iter.has_elements() {
				// the type is checked by the `MatIterMut::new()` and we ensure there are still elements by calling `has_elements()`
				let cur = unsafe { convert_ptr_mut(iter.ptr().cast_mut()) };
				let pos = iter.pos().ok()?;
				iter.seek(1, true).ok()?;
				Some((pos, cur))
			} else {
				None
			}
		})
	}
}

pub(crate) mod mat_forward {
	use super::*;

	#[inline]
	pub fn at<T: DataType>(mat: &(impl MatTraitConst + ?Sized), i0: i32) -> Result<&T> {
		match_format::<T>(mat.typ())
			.and_then(|_| match_total(mat, i0))
			.and_then(|_| unsafe { mat.at_unchecked(i0) })
	}

	#[inline]
	pub fn at_def<T: DataType>(mat: &(impl MatTraitConst + ?Sized)) -> Result<&T> {
		at(mat, 0)
	}

	#[inline]
	pub fn at_mut<T: DataType>(mat: &mut (impl MatTrait + ?Sized), i0: i32) -> Result<&mut T> {
		match_format::<T>(mat.typ()).and_then(|_| match_total(mat, i0))?;
		unsafe { mat.at_unchecked_mut(i0) }
	}

	#[inline]
	pub fn at_mut_def<T: DataType>(mat: &mut (impl MatTrait + ?Sized)) -> Result<&mut T> {
		at_mut(mat, 0)
	}

	#[inline]
	pub fn at_2d<T: DataType>(mat: &(impl MatTraitConst + ?Sized), row: i32, col: i32) -> Result<&T> {
		match_format::<T>(mat.typ())
			.and_then(|_| match_indices(mat, &[row, col]))
			.and_then(|_| unsafe { mat.at_2d_unchecked(row, col) })
	}

	#[inline]
	pub fn at_2d_mut<T: DataType>(mat: &mut (impl MatTrait + ?Sized), row: i32, col: i32) -> Result<&mut T> {
		match_format::<T>(mat.typ()).and_then(|_| match_indices(mat, &[row, col]))?;
		unsafe { mat.at_2d_unchecked_mut(row, col) }
	}

	#[inline]
	pub fn at_pt<T: DataType>(mat: &(impl MatTraitConst + ?Sized), pt: Point) -> Result<&T> {
		at_2d(mat, pt.y, pt.x)
	}

	#[inline]
	pub fn at_pt_mut<T: DataType>(mat: &mut (impl MatTrait + ?Sized), pt: Point) -> Result<&mut T> {
		at_2d_mut(mat, pt.y, pt.x)
	}

	#[inline]
	pub fn at_3d<T: DataType>(mat: &(impl MatTraitConst + ?Sized), i0: i32, i1: i32, i2: i32) -> Result<&T> {
		match_format::<T>(mat.typ())
			.and_then(|_| match_indices(mat, &[i0, i1, i2]))
			.and_then(|_| unsafe { mat.at_3d_unchecked(i0, i1, i2) })
	}

	#[inline]
	pub fn at_3d_mut<T: DataType>(mat: &mut (impl MatTrait + ?Sized), i0: i32, i1: i32, i2: i32) -> Result<&mut T> {
		match_format::<T>(mat.typ()).and_then(|_| match_indices(mat, &[i0, i1, i2]))?;
		unsafe { mat.at_3d_unchecked_mut(i0, i1, i2) }
	}

	#[inline]
	pub fn at_nd<'s, T: DataType>(mat: &'s (impl MatTraitConst + ?Sized), idx: &[i32]) -> Result<&'s T> {
		match_format::<T>(mat.typ())
			.and_then(|_| match_indices(mat, idx))
			.and_then(|_| unsafe { mat.at_nd_unchecked(idx) })
	}

	#[inline]
	pub fn at_nd_mut<'s, T: DataType>(mat: &'s mut (impl MatTrait + ?Sized), idx: &[i32]) -> Result<&'s mut T> {
		match_format::<T>(mat.typ()).and_then(|_| match_indices(mat, idx))?;
		unsafe { mat.at_nd_unchecked_mut(idx) }
	}
}

pub trait MatTraitConstManual: MatTraitConst {
	/// Like `Mat::at()` but performs no bounds or type checks
	///
	/// # Safety
	/// Caller must ensure that index is within Mat bounds
	#[inline]
	unsafe fn at_unchecked<T: DataType>(&self, i0: i32) -> Result<&T> {
		let mat_size = self.size()?;
		let (i, j) = if self.is_continuous() || mat_size.width == 1 {
			(0, i0)
		} else if mat_size.height == 1 {
			(i0, 0)
		} else {
			let i = i0 / mat_size.height;
			(i, i0 - i * mat_size.height)
		};
		self.ptr_2d(i, j).map(|ptr| unsafe { convert_ptr(ptr) })
	}

	/// Like `Mat::at_2d()` but performs no bounds or type checks
	/// # Safety
	/// Caller must ensure that indices are within Mat bounds
	#[inline]
	unsafe fn at_2d_unchecked<T: DataType>(&self, row: i32, col: i32) -> Result<&T> {
		self.ptr_2d(row, col).map(|ptr| unsafe { convert_ptr(ptr) })
	}

	/// Like `Mat::at_pt()` but performs no bounds or type checks
	/// # Safety
	/// Caller must ensure that point is within Mat bounds
	#[inline]
	unsafe fn at_pt_unchecked<T: DataType>(&self, pt: Point) -> Result<&T> {
		unsafe { self.at_2d_unchecked(pt.y, pt.x) }
	}

	/// Like `Mat::at_3d()` but performs no bounds or type checks
	/// # Safety
	/// Caller must ensure that indices are within Mat bounds
	#[inline]
	unsafe fn at_3d_unchecked<T: DataType>(&self, i0: i32, i1: i32, i2: i32) -> Result<&T> {
		self.ptr_3d(i0, i1, i2).map(|ptr| unsafe { convert_ptr(ptr) })
	}

	/// Like `Mat::at_nd()` but performs no bounds or type checks
	/// # Safety
	/// Caller must ensure that indices are within Mat bounds
	#[inline]
	unsafe fn at_nd_unchecked<T: DataType>(&self, idx: &[i32]) -> Result<&T> {
		self.ptr_nd(idx).map(|ptr| unsafe { convert_ptr(ptr) })
	}

	/// Return a complete read-only row
	#[inline]
	fn at_row<T: DataType>(&self, row: i32) -> Result<&[T]> {
		match_format::<T>(self.typ())
			.and_then(|_| match_indices(self, &[row, 0]))
			.and_then(|_| unsafe { self.at_row_unchecked(row) })
	}

	/// Like `Mat::at_row()` but performs no bounds or type checks
	/// # Safety
	/// Caller must ensure that index is within Mat bounds
	#[inline]
	unsafe fn at_row_unchecked<T: DataType>(&self, row: i32) -> Result<&[T]> {
		// safe because Mat::size() can't be negative
		let width = self.size()?.width as usize;
		self.ptr(row).map(|row| {
			if row.is_null() {
				&[]
			} else {
				unsafe { slice::from_raw_parts(convert_ptr(row), width) }
			}
		})
	}

	#[inline]
	fn is_allocated(&self) -> bool {
		!self.data().is_null()
	}

	/// Returns underlying data array as byte slice, `Mat` must be continuous
	#[inline]
	fn data_bytes(&self) -> Result<&[u8]> {
		match_is_continuous(self).and_then(|_| {
			let data = self.data();
			Ok(if data.is_null() {
				&[]
			} else {
				unsafe { slice::from_raw_parts(data, self.total() * self.elem_size()?) }
			})
		})
	}

	#[inline]
	fn data_typed<T: DataType>(&self) -> Result<&[T]> {
		match_format::<T>(self.typ())
			.and_then(|_| match_is_continuous(self))
			.and_then(|_| unsafe { self.data_typed_unchecked() })
	}

	/// # Safety
	/// Caller must ensure that the `T` type argument corresponds to the data stored in the `Mat` and `Mat` is continuous
	#[inline]
	unsafe fn data_typed_unchecked<T: DataType>(&self) -> Result<&[T]> {
		let data = self.data();
		Ok(if data.is_null() {
			&[]
		} else {
			unsafe { slice::from_raw_parts(data.cast::<T>(), self.total()) }
		})
	}

	fn to_vec_2d<T: DataType>(&self) -> Result<Vec<Vec<T>>> {
		match_format::<T>(self.typ()).and_then(|_| {
			let size = match *self.mat_size() {
				[rows, cols] => Size::new(cols, rows),
				ref mat_size => {
					return Err(Error::new(
						core::StsUnmatchedSizes,
						format!(
							"Mat must have 2 dimensions for this operation, but it has: {}",
							mat_size.len()
						),
					))
				}
			};
			// safe because Mat size can't be negative
			let width = size.width as usize;
			if self.is_continuous() {
				let data = self.data_typed()?;
				Ok((0..size.height)
					.map(|row_n| {
						// safe because the iteration starts from 0
						let row_n = row_n as usize;
						let mut row = Vec::with_capacity(width);
						row.extend_from_slice(&data[row_n * width..(row_n + 1) * width]);
						row
					})
					.collect())
			} else {
				Ok((0..size.height)
					.map(|row_n| {
						self.at_row(row_n).map(|src_row| {
							let mut row = Vec::with_capacity(width);
							row.extend_from_slice(src_row);
							row
						})
					})
					.collect::<Result<_>>()?)
			}
		})
	}

	/// Returns an iterator over `Mat` elements and their positions
	#[inline]
	fn iter<T: DataType>(&self) -> Result<MatIter<T>>
	where
		Self: Sized,
	{
		MatConstIterator::over(self).map_or(
			Ok(MatIter {
				iter: None,
				_d: PhantomData,
			}),
			MatIter::new,
		)
	}

	#[inline]
	fn try_into_typed<T: DataType>(self) -> Result<Mat_<T>>
	where
		Self: Sized,
		Mat_<T>: TryFrom<Self, Error = Error>,
	{
		self.try_into()
	}
}

pub trait MatTraitManual: MatTraitConstManual + MatTrait {
	/// Like `Mat::at_mut()` but performs no bounds or type checks
	/// # Safety
	/// Caller must ensure that index is within Mat bounds
	#[inline]
	unsafe fn at_unchecked_mut<T: DataType>(&mut self, i0: i32) -> Result<&mut T> {
		let (i, j) = idx_to_row_col(self, i0)?;
		self.ptr_2d_mut(i, j).map(|ptr| unsafe { convert_ptr_mut(ptr) })
	}

	/// Like `Mat::at_2d_mut()` but performs no bounds or type checks
	/// # Safety
	/// Caller must ensure that indices are within Mat bounds
	#[inline]
	unsafe fn at_2d_unchecked_mut<T: DataType>(&mut self, row: i32, col: i32) -> Result<&mut T> {
		self.ptr_2d_mut(row, col).map(|ptr| unsafe { convert_ptr_mut(ptr) })
	}

	/// Like `Mat::at_pt_mut()` but performs no bounds or type checks
	/// # Safety
	/// Caller must ensure that point is within Mat bounds
	#[inline]
	unsafe fn at_pt_unchecked_mut<T: DataType>(&mut self, pt: Point) -> Result<&mut T> {
		unsafe { self.at_2d_unchecked_mut(pt.y, pt.x) }
	}

	/// Like `Mat::at_3d_mut()` but performs no bounds or type checks
	/// # Safety
	/// Caller must ensure that indices are within Mat bounds
	#[inline]
	unsafe fn at_3d_unchecked_mut<T: DataType>(&mut self, i0: i32, i1: i32, i2: i32) -> Result<&mut T> {
		self.ptr_3d_mut(i0, i1, i2).map(|ptr| unsafe { convert_ptr_mut(ptr) })
	}

	/// Like `Mat::at_nd_mut()` but performs no bounds or type checks
	/// # Safety
	/// Caller must ensure that indices are within Mat bounds
	#[inline]
	unsafe fn at_nd_unchecked_mut<T: DataType>(&mut self, idx: &[i32]) -> Result<&mut T> {
		self.ptr_nd_mut(idx).map(|ptr| unsafe { convert_ptr_mut(ptr) })
	}

	/// Return a complete writeable row
	#[inline]
	fn at_row_mut<T: DataType>(&mut self, row: i32) -> Result<&mut [T]> {
		match_format::<T>(self.typ()).and_then(|_| match_indices(self, &[row, 0]))?;
		unsafe { self.at_row_unchecked_mut(row) }
	}

	/// Like `Mat::at_row_mut()` but performs no bounds or type checks
	/// # Safety
	/// Caller must ensure that index is within Mat bounds
	#[inline]
	unsafe fn at_row_unchecked_mut<T: DataType>(&mut self, row: i32) -> Result<&mut [T]> {
		// safe because Mat::size() can't be negative
		let width = self.size()?.width as usize;
		self.ptr_mut(row).map(|x| {
			if x.is_null() {
				&mut []
			} else {
				unsafe { slice::from_raw_parts_mut(convert_ptr_mut(x), width) }
			}
		})
	}

	/// Returns underlying data array as mutable byte slice, Mat must be continuous.
	#[inline]
	fn data_bytes_mut(&mut self) -> Result<&mut [u8]> {
		match_is_continuous(self).and_then(|_| {
			let data = self.data_mut();
			Ok(if data.is_null() {
				&mut []
			} else {
				unsafe { slice::from_raw_parts_mut(self.data_mut(), self.total() * self.elem_size()?) }
			})
		})
	}

	#[inline]
	fn data_typed_mut<T: DataType>(&mut self) -> Result<&mut [T]> {
		match_format::<T>(self.typ()).and_then(|_| match_is_continuous(self))?;
		unsafe { self.data_typed_unchecked_mut() }
	}

	/// # Safety
	/// Caller must ensure that the `T` type argument corresponds to the data stored in the `Mat` and `Mat` is continuous
	#[inline]
	unsafe fn data_typed_unchecked_mut<T: DataType>(&mut self) -> Result<&mut [T]> {
		let total = self.total();
		let data = self.data_mut();
		Ok(if data.is_null() {
			&mut []
		} else {
			unsafe { slice::from_raw_parts_mut(data.cast::<T>(), total) }
		})
	}

	/// Returns a mutable iterator over `Mat` elements and their positions
	#[inline]
	fn iter_mut<T: DataType>(&mut self) -> Result<MatIterMut<T>>
	where
		Self: Sized,
	{
		MatConstIterator::over(self).map_or(
			Ok(MatIterMut {
				iter: None,
				_d: PhantomData,
			}),
			MatIterMut::new,
		)
	}
}

impl<T: MatTraitConst + ?Sized> MatTraitConstManual for T {}

impl<T: MatTrait + ?Sized> MatTraitManual for T {}

input_output_array! { Mat, from_mat, from_mat_mut }
input_output_array_vector! { Mat, from_mat_vec, from_mat_vec_mut }

unsafe impl Sync for Mat {}

impl fmt::Debug for Mat {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		let typ = self.typ();
		let depth = self.depth();
		let typ = core::type_to_string(typ).map_err(|_| fmt::Error)?;
		let depth = core::depth_to_string(depth).map_err(|_| fmt::Error)?;
		let flags = self.flags();
		let mut flags_str = String::new();
		if flags & core::Mat_MAGIC_VAL != core::Mat_MAGIC_VAL {
			flags_str.push_str("invalid magic value");
		} else {
			if flags & core::Mat_CONTINUOUS_FLAG != 0 {
				flags_str.push_str("continuous");
			}
			if flags & core::Mat_SUBMATRIX_FLAG != 0 {
				if !flags_str.is_empty() {
					flags_str.push_str(", ");
				}
				flags_str.push_str("submatrix");
			}
		}
		f.debug_struct("Mat")
			.field("type", &typ)
			.field("flags", &format!("0x{flags:X} ({flags_str})"))
			.field("channels", &self.channels())
			.field("depth", &depth)
			.field("dims", &self.dims())
			.field("size", &self.size().map_err(|_| fmt::Error)?)
			.field("rows", &self.rows())
			.field("cols", &self.cols())
			.field("elem_size", &self.elem_size().map_err(|_| fmt::Error)?)
			.field("elem_size1", &self.elem_size1())
			.field("total", &self.total())
			.field("is_continuous", &self.is_continuous())
			.field("is_submatrix", &self.is_submatrix())
			.field("data", &MatDataDumper(self))
			.finish()
	}
}

struct MatDataDumper<'r>(&'r Mat);

impl fmt::Debug for MatDataDumper<'_> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		const MAX_DUMP_SIZE: usize = 1000;

		if self.0.total() <= MAX_DUMP_SIZE {
			f.write_str(&self.0.get_data_dump().map_err(|_| fmt::Error)?)
		} else {
			f.write_fmt(format_args!("<element count is higher than threshold: {MAX_DUMP_SIZE}>"))
		}
	}
}

input_output_array! { UMat, from_umat, from_umat_mut }
input_output_array_vector! { UMat, from_umat_vec, from_umat_vec_mut }

impl Deref for MatSize<'_> {
	type Target = [i32];

	#[inline]
	fn deref(&self) -> &Self::Target {
		let p = self.p();
		if p.is_null() {
			&[]
		} else {
			let dims = usize::try_from(self.dims()).unwrap_or(0);
			unsafe { slice::from_raw_parts(p, dims) }
		}
	}
}

impl fmt::Debug for MatSize<'_> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		writeln!(f, "{:#?}", self.deref())
	}
}

pub trait MatConstIteratorTraitManual: MatConstIteratorTrait {
	#[inline]
	fn has_elements(&self) -> bool {
		self.ptr() != self.slice_end()
	}

	#[inline]
	fn current<T: DataType>(&self) -> Result<&T> {
		match_format::<T>(self.typ())?;
		if self.has_elements() {
			Ok(unsafe { convert_ptr(self.ptr()) })
		} else {
			Err(Error::new(
				core::StsOutOfRange,
				"MatConstIterator doesn't have any more elements",
			))
		}
	}

	#[inline]
	fn current_mut<T: DataType>(&mut self) -> Result<&mut T> {
		match_format::<T>(self.typ())?;
		if self.has_elements() {
			Ok(unsafe { convert_ptr_mut(self.ptr().cast_mut()) })
		} else {
			Err(Error::new(
				core::StsOutOfRange,
				"MatConstIterator doesn't have any more elements",
			))
		}
	}
}

impl<T: MatConstIteratorTrait> MatConstIteratorTraitManual for T {}

input_output_array! { MatExpr, from_matexpr }
