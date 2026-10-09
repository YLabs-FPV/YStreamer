import type { ViewerTransport } from "@/types/settings";

export type Access = "none" | "viewer" | "admin";

export interface AuthStatus {
  enabled: boolean;
  protect_viewing: boolean;
  access: Access;
  transport: ViewerTransport;
}
