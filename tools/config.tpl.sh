opencv_lib_base_dir="<home of the separate opencv install>"
BINDINGS_OUT_DIR="$opencv_lib_base_dir/bindings"

# 3.4
OPENCV_34_CMAKE_DIR="$opencv_lib_base_dir/opencv-3.4/install/share/OpenCV"
OPENCV_34_HEADER_DIR="$opencv_lib_base_dir/opencv-3.4/install/include/"
OPENCV_34_LD_LIBRARY_PATH="$opencv_lib_base_dir/opencv-3.4/install/lib64/"
OPENCV_34_ADDITIONAL_INCLUDE_DIRS=""

# 4.x
OPENCV_4_CMAKE_DIR="$opencv_lib_base_dir/opencv-4/install/lib64/cmake/opencv4"
OPENCV_4_HEADER_DIR="$opencv_lib_base_dir/opencv-4/install/include/opencv4"
OPENCV_4_LD_LIBRARY_PATH="$opencv_lib_base_dir/opencv-4/install/lib64/"
OPENCV_4_ADDITIONAL_INCLUDE_DIRS=""

# 5.x
OPENCV_5_CMAKE_DIR="$opencv_lib_base_dir/opencv-5/install/lib64/cmake/opencv5"
OPENCV_5_HEADER_DIR="$opencv_lib_base_dir/opencv-5/install/include/opencv5"
OPENCV_5_LD_LIBRARY_PATH="$opencv_lib_base_dir/opencv-5/install/lib64/"
OPENCV_5_ADDITIONAL_INCLUDE_DIRS=""

# other OS machines
MACOS_ADDR="<ssh address for macos machine>"
WIN_ADDR="<ssh address for win machine>"
