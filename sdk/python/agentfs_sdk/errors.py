"""Error types for filesystem operations"""

from typing import Literal, Optional

# POSIX-style error codes for filesystem operations
FsErrorCode = Literal[
    "ENOENT",    # No such file or directory
    "EEXIST",    # File already exists
    "EISDIR",    # Is a directory (when file expected)
    "ENOTDIR",   # Not a directory (when directory expected)
    "ENOTEMPTY", # Directory not empty
    "EPERM",     # Operation not permitted
    "EINVAL",    # Invalid argument
    "ENOSYS",    # Function not implemented (use for symlinks)
]

# Filesystem syscall names for error reporting
# rm, scandir and copyfile are not actual syscalls but used for convenience
FsSyscall = Literal[
    "open",
    "stat",
    "mkdir",
    "rmdir",
    "rm",
    "unlink",
    "rename",
    "scandir",
    "copyfile",
    "access",
]


class ErrnoException(Exception):
    """Exception with errno-style attributes"""

    def __init__(
        self,
        message: str,
        code: Optional[FsErrorCode] = None,
        syscall: Optional[FsSyscall] = None,
        path: Optional[str] = None,
    ):
        super().__init__(message)
        self.code = code
        self.syscall = syscall
        self.path = path


def create_fs_error(
    code: FsErrorCode,
    syscall: FsSyscall,
    path: Optional[str] = None,
    message: Optional[str] = None,
) -> ErrnoException:
    """Create a filesystem error with consistent formatting

    Args:
        code: POSIX error code (e.g., 'ENOENT')
        syscall: System call name (e.g., 'open')
        path: Optional path involved in the error
        message: Optional custom message (defaults to code)

    Returns:
        ErrnoException with formatted message and attributes
    """
    base = message if message else code
    suffix = f" '{path}'" if path is not None else ""
    error_message = f"{code}: {base}, {syscall}{suffix}"

    # For ENOENT, also inherit from FileNotFoundError for backward compatibility
    if code == "ENOENT":
        # Create a custom exception class that inherits from both
        class FileNotFoundErrnoException(ErrnoException, FileNotFoundError):
            pass
        return FileNotFoundErrnoException(error_message, code=code, syscall=syscall, path=path)

    return ErrnoException(error_message, code=code, syscall=syscall, path=path)
