export type UserRole = "admin" | "operator";

export interface DashboardUser {
  id: string;
  email: string;
  password_hash: string;
  name: string;
  role: UserRole;
  active: boolean;
  created_at: string;
  updated_at: string;
  totp_state?: string;
  auth_version?: string;
}

export interface PublicUser {
  id: string;
  email: string;
  name: string;
  role: UserRole;
  active: boolean;
  created_at: string;
  updated_at: string;
  totp_enabled: boolean;
}

export interface CreateUserInput {
  email: string;
  password: string;
  name?: string;
  role?: UserRole;
  active?: boolean;
}

export interface UpdateUserInput {
  email?: string;
  name?: string;
  role?: UserRole;
  active?: boolean;
  password?: string;
}

export function toPublicUser(user: DashboardUser): PublicUser {
  return {
    id: user.id,
    email: user.email,
    name: user.name,
    role: user.role,
    active: user.active,
    created_at: user.created_at,
    updated_at: user.updated_at,
    totp_enabled: Boolean(JSON.parse(user.totp_state ?? "{}").secret),
  };
}
