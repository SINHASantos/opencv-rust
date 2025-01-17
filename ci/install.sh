#!/bin/bash

set -xeu

ci_dir="$(dirname "$0")"

if [[ "$OSTYPE" == "linux-gnu"* ]]; then
	os_family="Linux"
elif [[ "$OSTYPE" == "darwin"* ]]; then
	os_family="macOS"
elif [[ "$OSTYPE" == "cygwin" || "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
	os_family="Windows"
elif [[ "$OSTYPE" == "freebsd"* ]]; then
	exit "FreeBSD is not supported"
else
	exit "Unknown OS: $OSTYPE"
fi

if [[ "$os_family" == "Linux" ]]; then
	# free up disk space in Github Actions image: https://github.com/actions/runner-images/issues/2840
	sudo rm -rf /usr/share/dotnet /opt/ghc /usr/local/share/boost
	if [[ "${VCPKG_VERSION:-}" != "" ]]; then # vcpkg build
		"$ci_dir/install-ubuntu-vcpkg.sh"
	else
		# workaround for mozilla/sccache action problem /bin/sh: 1: sccache: not found when running `sudo make install`
		if [[ "${CMAKE_C_COMPILER_LAUNCHER:-}" == "sccache" ]]; then
			export CMAKE_C_COMPILER_LAUNCHER="$(which sccache)"
		fi
		if [[ "${CMAKE_CXX_COMPILER_LAUNCHER:-}" == "sccache" ]]; then
			export CMAKE_CXX_COMPILER_LAUNCHER="$(which sccache)"
		fi
		"$ci_dir/install-ubuntu.sh"
	fi
elif [[ "$os_family" == "macOS" ]]; then
	if [[ "${BREW_OPENCV_VERSION:-}" != "" ]]; then # brew build
		"$ci_dir/install-macos-brew.sh"
	elif [[ "${VCPKG_VERSION:-}" != "" ]]; then # vcpkg build
		"$ci_dir/install-macos-vcpkg.sh"
	else
		"$ci_dir/install-macos-framework.sh"
	fi
elif [[ "$os_family" == "Windows" ]]; then
	export CHOCO_LLVM_VERSION=19.1.5
	if [[ "${VCPKG_VERSION:-}" != "" ]]; then # vcpkg build
		"$ci_dir/install-windows-vcpkg.sh"
	else # chocolatey build
		"$ci_dir/install-windows-chocolatey.sh"
	fi
fi
