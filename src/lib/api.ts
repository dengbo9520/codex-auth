import { invoke } from "@tauri-apps/api/core";

import type {
  AppSnapshotDto,
  CommandExecutionDto,
  MutationResultDto,
} from "@/lib/types";

export const api = {
  getAppSnapshot() {
    return invoke<AppSnapshotDto>("get_app_snapshot");
  },
  refreshRegistrySnapshot() {
    return invoke<MutationResultDto>("refresh_registry_snapshot");
  },
  runCodexAuthStatus() {
    return invoke<CommandExecutionDto>("run_codex_auth_status");
  },
  switchAccount(query: string) {
    return invoke<MutationResultDto>("switch_account", { query });
  },
  removeAccount(query: string) {
    return invoke<MutationResultDto>("remove_account", { query });
  },
  setAccountAlias(accountKey: string, alias: string) {
    return invoke<MutationResultDto>("set_account_alias", {
      accountKey,
      alias,
    });
  },
  importAuthFile(path: string, alias?: string) {
    return invoke<MutationResultDto>("import_auth_file", {
      path,
      alias: alias ?? null,
    });
  },
  importAuthDirectory(path: string) {
    return invoke<MutationResultDto>("import_auth_directory", { path });
  },
  importCpa(path?: string, alias?: string) {
    return invoke<MutationResultDto>("import_cpa", {
      path: path ?? null,
      alias: alias ?? null,
    });
  },
  rebuildRegistry(path?: string) {
    return invoke<MutationResultDto>("rebuild_registry", {
      path: path ?? null,
    });
  },
  setAutoSwitch(enabled: boolean) {
    return invoke<MutationResultDto>("set_auto_switch", { enabled });
  },
  setUsageApiMode(enabled: boolean) {
    return invoke<MutationResultDto>("set_usage_api_mode", { enabled });
  },
  recordUiEvent(name: string, detail?: unknown) {
    return invoke<CommandExecutionDto>("record_ui_event", {
      name,
      detail: detail === undefined ? null : JSON.stringify(detail),
    });
  },
  launchLogin(deviceAuth: boolean) {
    return invoke<CommandExecutionDto>("launch_login", {
      deviceAuth,
    });
  },
  openDiagnosticPath(target: string) {
    return invoke<CommandExecutionDto>("open_diagnostic_path", { target });
  },
};
