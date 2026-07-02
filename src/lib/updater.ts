import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";

export type { Update };

// Version courante de l'app (depuis tauri.conf.json / Cargo.toml).
export function appVersion(): Promise<string> {
  return getVersion();
}

// Vérifie l'endpoint de mise à jour ; renvoie null si l'app est à jour.
export function checkForUpdate(): Promise<Update | null> {
  return check();
}

// Télécharge + installe la mise à jour, puis relance l'app.
export async function installUpdate(update: Update): Promise<void> {
  await update.downloadAndInstall();
  await relaunch();
}
