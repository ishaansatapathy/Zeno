import type { Client } from "./client";

export type Role = "student" | "creator" | "admin";

export type User = {
  id: string;
  name: string;
  email: string | null;
  role: Role;
  email_verified: boolean;
};

export type SignUpInput = { name: string; email: string; password: string; role: "student" | "creator" };

/** The typed calls the account pages make; the shapes mirror `crates/api/src/auth/dto.rs`. */
export function authApi(client: Client) {
  return {
    register: (input: SignUpInput) => client.post<{ message: string }>("/v1/auth/register", { body: input }),
    login: (email: string, password: string) =>
      client.post<{ user: User }>("/v1/auth/login", { body: { email, password } }),
    me: (signal?: AbortSignal) => client.get<{ user: User }>("/v1/auth/me", { signal }),
    logout: () => client.post<void>("/v1/auth/logout"),
  };
}
