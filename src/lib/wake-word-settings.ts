import type { AppSettings } from "../types";

export async function persistSuccessfulWakeCalibration(
  settings: AppSettings,
  persist: (settings: AppSettings) => Promise<void>,
): Promise<AppSettings> {
  const enabled = { ...settings, wakeWordEnabled: true };
  await persist(enabled);
  return enabled;
}
