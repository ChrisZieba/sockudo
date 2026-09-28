export type UserRole = "admin" | "operator";

export interface DashboardUser {
  id: string;
  email: string;
  name: string;
  role: UserRole;
  active: boolean;
  totp_enabled: boolean;
  created_at: string;
  updated_at: string;
}
