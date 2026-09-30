export class CommandError extends Error {
  override readonly name = "CommandError";
}

export function hasErrorCode(error: unknown, code: string): boolean {
  return error instanceof Error && "code" in error && error.code === code;
}
