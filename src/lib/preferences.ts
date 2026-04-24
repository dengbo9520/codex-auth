import { Store } from "@tauri-apps/plugin-store";

import type { PageKey } from "@/lib/types";

export interface AppPreferences {
  lastPage: PageKey;
  accountsSearch: string;
}

const DEFAULT_PREFERENCES: AppPreferences = {
  lastPage: "dashboard",
  accountsSearch: "",
};

let storePromise: Promise<Store> | null = null;

async function getStore() {
  if (!storePromise) {
    storePromise = Store.load("gui-preferences.json");
  }

  return storePromise;
}

export async function loadPreferences(): Promise<AppPreferences> {
  const store = await getStore();
  const saved = await store.get<AppPreferences>("preferences");
  return {
    ...DEFAULT_PREFERENCES,
    ...saved,
  };
}

export async function savePreferences(
  patch: Partial<AppPreferences>,
): Promise<AppPreferences> {
  const store = await getStore();
  const next = {
    ...(await loadPreferences()),
    ...patch,
  };

  await store.set("preferences", next);
  await store.save();
  return next;
}
