import { invoke } from "@tauri-apps/api/core";

function errorMessage(error: unknown): string {
  if (error instanceof Error) return `${error.name}: ${error.message}`;
  if (typeof error === "string") return error;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
}

export async function logAppError(operation: string, error: unknown): Promise<void> {
  try {
    await invoke("frontend_log_error", {
      operation: operation.slice(0, 120),
      message: errorMessage(error).slice(0, 2000)
    });
  } catch {
    // Logging must never replace or recursively trigger the original error path.
  }
}
