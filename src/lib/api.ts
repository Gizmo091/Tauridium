import { invoke } from "@tauri-apps/api/core";

// Modèles calqués sur l'API serveur Ferdium (/v1).
export interface MeUser {
  email: string;
  firstname: string;
  lastname: string;
  id: string;
  locale?: string;
  [k: string]: unknown;
}

export interface Service {
  id: string;
  name: string;
  recipeId: string;
  iconUrl: string | null;
  isEnabled: boolean;
  isMuted?: boolean;
  isNotificationEnabled?: boolean;
  isBadgeEnabled?: boolean;
  order?: number;
  workspaces?: string[];
  [k: string]: unknown;
}

export interface RecipePreview {
  id: string;
  name: string;
  icons?: { svg?: string };
  [k: string]: unknown;
}

export interface Workspace {
  id: string;
  name: string;
  order: number;
  services: string[];
  userId: number | string;
}

export const DEFAULT_SERVER = "https://api.ferdium.org";

// Toutes les requêtes HTTP partent du côté Rust (pas de CORS, token gardé hors du JS).
export function login(
  server: string,
  email: string,
  password: string,
): Promise<MeUser> {
  return invoke("login", { server, email, password });
}

export function getServices(): Promise<Service[]> {
  return invoke("get_services");
}

export function getWorkspaces(): Promise<Workspace[]> {
  return invoke("get_workspaces");
}

// Restaure une session enregistrée (rejette s'il n'y en a pas / token expiré).
export function restoreSession(): Promise<MeUser> {
  return invoke("restore_session");
}

export function logout(): Promise<void> {
  return invoke("logout");
}

// Phase 2 : affiche le service actif dans une webview enfant isolée.
export function showService(s: Service): Promise<void> {
  return invoke("show_service", {
    serviceId: s.id,
    recipeId: s.recipeId,
    customUrl: (s.customUrl as string | undefined) ?? null,
    team: (s.team as string | undefined) ?? null,
  });
}

export function closeServices(): Promise<void> {
  return invoke("close_services");
}

export function hideServices(): Promise<void> {
  return invoke("hide_all_services");
}

// Pousse les réglages d'un service (notif/mute/badge) que le poller Rust respecte.
export function setServiceFlags(s: Service): Promise<void> {
  return invoke("set_service_flags", {
    serviceId: s.id,
    notif: s.isNotificationEnabled !== false,
    muted: s.isMuted === true,
    badge: s.isBadgeEnabled !== false,
  });
}

export function updateService(
  serviceId: string,
  patch: Record<string, unknown>,
): Promise<unknown> {
  return invoke("update_service", { serviceId, patch });
}

export function createService(name: string, recipeId: string): Promise<unknown> {
  return invoke("create_service", { name, recipeId });
}

export function deleteService(serviceId: string): Promise<void> {
  return invoke("delete_service", { serviceId });
}

export function listRecipes(): Promise<RecipePreview[]> {
  return invoke("list_recipes");
}

export function createWorkspace(name: string): Promise<Workspace> {
  return invoke("create_workspace", { name });
}

export function updateWorkspace(
  workspaceId: string,
  name: string,
  services: string[],
): Promise<Workspace> {
  return invoke("update_workspace", { workspaceId, name, services });
}

export function deleteWorkspace(workspaceId: string): Promise<void> {
  return invoke("delete_workspace", { workspaceId });
}

export interface AppSettings {
  autostart: boolean;
  startMinimized: boolean;
  theme: "dark" | "light" | "system";
  [k: string]: unknown;
}

export function getAppSettings(): Promise<AppSettings> {
  return invoke("get_app_settings");
}

export function setAppSettings(
  patch: Partial<AppSettings>,
): Promise<AppSettings> {
  return invoke("set_app_settings", { patch });
}

// Ouvre les devtools sur la webview du service actif (debug).
export function inspectService(): Promise<void> {
  return invoke("inspect_service");
}
