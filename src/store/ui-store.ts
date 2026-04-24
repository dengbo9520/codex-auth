import { create } from "zustand";

import { loadPreferences, savePreferences } from "@/lib/preferences";
import type { PageKey } from "@/lib/types";

interface UiStoreState {
  initialized: boolean;
  page: PageKey;
  accountsSearch: string;
  initialize: () => Promise<void>;
  setPage: (page: PageKey) => void;
  setAccountsSearch: (value: string) => void;
}

export const useUiStore = create<UiStoreState>((set, get) => ({
  initialized: false,
  page: "dashboard",
  accountsSearch: "",
  async initialize() {
    if (get().initialized) {
      return;
    }

    const preferences = await loadPreferences();
    set({
      initialized: true,
      page: preferences.lastPage,
      accountsSearch: preferences.accountsSearch,
    });
  },
  setPage(page) {
    set({ page });
    void savePreferences({ lastPage: page });
  },
  setAccountsSearch(value) {
    set({ accountsSearch: value });
    void savePreferences({ accountsSearch: value });
  },
}));
