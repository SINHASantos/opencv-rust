use opencv::core::Vec3b;
use opencv::prelude::*;
use opencv::{Result, imgcodecs};
use tempdir::TempDir;

#[test]
#[cfg_attr(windows, ignore)]
// test ignored on Windows because Unicode filenames are not supported by OpenCV there
fn test_unicode_filename() -> Result<()> {
	static DEMO_PNG: &[u8] = include_bytes!("pixel.png");

	let write_img = imgcodecs::imdecode(&DEMO_PNG, imgcodecs::IMREAD_COLOR)?;
	let unicode_filename = "Ελλάδα.png";
	let tmp_dir = TempDir::new("ocvrs_unicode_path_test").expect("Can't create temp dir");
	let unicode_file_path = tmp_dir.path().join(unicode_filename);
	imgcodecs::imwrite_def(&unicode_file_path, &write_img)?;
	assert!(
		unicode_file_path.exists(),
		"File {} doesn't exist",
		unicode_file_path.display()
	);

	let read_back_img = imgcodecs::imread_def(unicode_file_path)?;
	assert_eq!(write_img.data_typed::<Vec3b>()?, read_back_img.data_typed::<Vec3b>()?);

	Ok(())
}
