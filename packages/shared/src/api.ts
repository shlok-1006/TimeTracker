import { z } from "zod";

/** Response shape of `GET /health`. */
export const healthSchema = z.object({
  status: z.literal("ok"),
});
export type Health = z.infer<typeof healthSchema>;

/** Standard API error envelope returned by the server (see `error.rs`). */
export const apiErrorSchema = z.object({
  error: z.string(),
});
export type ApiError = z.infer<typeof apiErrorSchema>;
