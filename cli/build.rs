fn main() {
    // On Linux, libunwind-ptrace.so may depend on liblzma.
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-link-lib=lzma");
        // libgcc_s provides _Unwind_RaiseException and other exception handling symbols
        println!("cargo:rustc-link-lib=dylib=gcc_s");
    }
}
