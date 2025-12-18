export interface ErrnoException extends Error {
  code?: string;
  syscall?: string;
  path?: string;
}

export function createFsError(params: {
  code: string;
  syscall: string;
  path?: string;
  message?: string;
}): ErrnoException {
  const { code, syscall, path, message } = params;
  const base = message ?? code;
  const suffix =
    path !== undefined
      ? ` '${path}'`
      : '';
  const err = new Error(`${code}: ${base}, ${syscall}${suffix}`) as ErrnoException;
  err.code = code;
  err.syscall = syscall;
  if (path !== undefined) err.path = path;
  return err;
}