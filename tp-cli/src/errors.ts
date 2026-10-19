// Base class for errors whose message is shown to the user as-is
export class TpError extends Error {}

export class CommandError extends TpError {
  override readonly name = "CommandError";
}

export class StoreError extends TpError {
  override readonly name = "StoreError";
}

export const errorCode = (error: unknown): string | undefined =>
  error instanceof Error && "code" in error && typeof error.code === "string" ? error.code : undefined;

export const hasErrorCode = (error: unknown, code: string): boolean => errorCode(error) === code;
