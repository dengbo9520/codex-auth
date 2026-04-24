import { useEffect } from "react";

import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";

import type { RegistryChangedEventDto } from "@/lib/types";

export function useRegistryEvents() {
  const queryClient = useQueryClient();

  useEffect(() => {
    let cleanup: (() => void) | undefined;

    void listen<RegistryChangedEventDto>("registry-changed", () => {
      void queryClient.invalidateQueries({ queryKey: ["appSnapshot"] });
    }).then((unlisten) => {
      cleanup = unlisten;
    });

    return () => {
      cleanup?.();
    };
  }, [queryClient]);
}
