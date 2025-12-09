fn main() {
    // Sandbox uses libunwind-ptrace which depends on liblzma and gcc_s.
    // Only available on Linux x86_64.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        println!("cargo:rustc-link-lib=lzma");
        // libgcc_s provides _Unwind_RaiseException and other exception handling symbols
        println!("cargo:rustc-link-lib=dylib=gcc_s");
    }

    // macOS: Weak-link libfuse so the binary can load without macFUSE installed.
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-arg=-Wl,-weak-lfuse");
        println!("cargo:rustc-link-search=/usr/local/lib");
        println!("cargo:rustc-link-search=/Library/Frameworks/macFUSE.framework/Versions/A");
    }
}
