export type PageKey =
  | "dashboard"
  | "accounts"
  | "import"
  | "settings"
  | "diagnostics";

export interface UsageWindowDto {
  usedPercent: number | null;
  remainingPercent: number | null;
  windowMinutes: number | null;
  resetsAtMs: number | null;
}

export interface AccountDto {
  accountKey: string;
  chatgptAccountId: string | null;
  chatgptUserId: string | null;
  email: string;
  alias: string;
  accountName: string | null;
  plan: string;
  authMode: string;
  active: boolean;
  createdAtMs: number | null;
  lastUsedAtMs: number | null;
  lastUsageAtMs: number | null;
  lastLocalRolloutMs: number | null;
  authStatus: string;
  authStatusCode: number | null;
  authStatusDetail: string | null;
  authCheckedAtMs: number | null;
  loginExpiresAtMs: number | null;
  authLastRefresh: string | null;
  authHasRefreshToken: boolean;
  subscriptionActiveUntil: string | null;
  subscriptionLastChecked: string | null;
  subscriptionPlan: string | null;
  verificationState: string | null;
  verificationLabel: string | null;
  verificationDetail: string | null;
  verificationCheckedAtMs: number | null;
  primaryUsage: UsageWindowDto | null;
  weeklyUsage: UsageWindowDto | null;
}

export interface RegistrySnapshotDto {
  schemaVersion: number | null;
  registryPath: string;
  accountsDir: string;
  activeAccountKey: string | null;
  activeAccountActivatedAtMs: number | null;
  autoSwitchEnabled: boolean;
  usageMode: string;
  accountApiEnabled: boolean;
  accounts: AccountDto[];
  warnings: string[];
}

export interface EnvCheckDto {
  key: string;
  label: string;
  ok: boolean;
  path: string | null;
  version: string | null;
  message: string;
}

export interface PerformanceSpanDto {
  label: string;
  durationMs: number;
  detail: string;
  timestampMs: number;
}

export interface DashboardSnapshotDto {
  activeAccount: AccountDto | null;
  remaining5hPercent: number | null;
  remainingWeeklyPercent: number | null;
  usageMode: string;
  autoSwitchEnabled: boolean;
  dataFreshness: string;
  envChecks: EnvCheckDto[];
  warnings: string[];
}

export interface DirectorySetDto {
  codexRoot: string;
  accountsDir: string;
  sessionsDir: string;
  registryPath: string;
  appLogDir: string;
  appLogFile: string;
}

export interface CommandExecutionDto {
  id: string;
  category: string;
  executablePath: string;
  displayCommand: string;
  args: string[];
  cwd: string;
  startedAtMs: number;
  finishedAtMs: number;
  durationMs: number;
  exitCode: number | null;
  success: boolean;
  timedOut: boolean;
  stdout: string;
  stderr: string;
}

export interface DiagnosticsSnapshotDto {
  envChecks: EnvCheckDto[];
  directories: DirectorySetDto;
  recentLogs: CommandExecutionDto[];
  latestStatusLog: CommandExecutionDto | null;
  performance: PerformanceSpanDto[];
}

export interface AppSnapshotDto {
  registry: RegistrySnapshotDto;
  dashboard: DashboardSnapshotDto;
  diagnostics: DiagnosticsSnapshotDto;
}

export interface MutationResultDto {
  command: CommandExecutionDto;
  registry: RegistrySnapshotDto;
}

export interface AccountVerificationDto {
  command: CommandExecutionDto;
  registry: RegistrySnapshotDto;
  accountKey: string;
  state: string;
  label: string;
  detail: string;
  switchedBack: boolean;
}

export interface RegistryChangedEventDto {
  kind: string;
  paths: string[];
  timestampMs: number;
}
