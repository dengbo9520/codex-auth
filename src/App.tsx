import {
  useDeferredValue,
  useEffect,
  useEffectEvent,
  useRef,
  useState,
} from "react";

import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQuery } from "@tanstack/react-query";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import {
  AlertCircleIcon,
  FolderOpenIcon,
  GaugeIcon,
  HardDriveDownloadIcon,
  InfoIcon,
  KeyRoundIcon,
  LaptopMinimalCheckIcon,
  Loader2Icon,
  LogsIcon,
  PencilIcon,
  RefreshCcwIcon,
  SearchIcon,
  Settings2Icon,
  ShieldAlertIcon,
  Trash2Icon,
  UserCog2Icon,
  WrenchIcon,
} from "lucide-react";
import { useForm } from "react-hook-form";
import { toast } from "sonner";
import { z } from "zod";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Progress, ProgressLabel } from "@/components/ui/progress";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarRail,
  SidebarTrigger,
} from "@/components/ui/sidebar";
import { Skeleton } from "@/components/ui/skeleton";
import { Toaster } from "@/components/ui/sonner";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useRegistryEvents } from "@/hooks/use-registry-events";
import { api } from "@/lib/api";
import {
  fallbackText,
  formatIsoRelative,
  formatIsoTimestamp,
  formatPercent,
  formatRelative,
  formatShortTimestamp,
  formatTimestamp,
} from "@/lib/format";
import type {
  AccountVerificationDto,
  AppSnapshotDto,
  CommandExecutionDto,
  MutationResultDto,
  PageKey,
} from "@/lib/types";
import { useUiStore } from "@/store/ui-store";

const importFileSchema = z.object({
  path: z.string().trim().min(1, "请选择账号 JSON 文件"),
  alias: z.string().trim().max(64, "别名最多 64 个字符").optional(),
});

const importDirectorySchema = z.object({
  path: z.string().trim().min(1, "请选择账号目录"),
});

const importCpaSchema = z.object({
  path: z.string().trim().optional(),
  alias: z.string().trim().max(64, "别名最多 64 个字符").optional(),
});

const rebuildRegistrySchema = z.object({
  path: z.string().trim().optional(),
});

const NAV_ITEMS: Array<{
  key: PageKey;
  label: string;
  icon: typeof GaugeIcon;
}> = [
  { key: "dashboard", label: "总览", icon: GaugeIcon },
  { key: "accounts", label: "账号", icon: UserCog2Icon },
  { key: "import", label: "导入", icon: HardDriveDownloadIcon },
  { key: "settings", label: "设置", icon: Settings2Icon },
  { key: "diagnostics", label: "诊断", icon: WrenchIcon },
];

const PAGE_META: Record<
  PageKey,
  { eyebrow: string; title: string; description: string }
> = {
  dashboard: {
    eyebrow: "激活账号 / 配额 / 环境",
    title: "本机运行总览",
    description: "读本机 registry 与 codex-auth 快照。只做 GUI，不重写认证。",
  },
  accounts: {
    eyebrow: "检索 / 切换 / 删除",
    title: "账号操作",
    description: "所有写操作只走 codex-auth CLI 非交互命令。",
  },
  import: {
    eyebrow: "JSON / 目录 / CPA / purge",
    title: "导入与重建",
    description: "导入后不会自动切换。去账号页手动切换。",
  },
  settings: {
    eyebrow: "自动切换 / API 模式 / 登录",
    title: "运行设置",
    description: "网页登录走外部窗口，并按 Codex 提示拉起浏览器。",
  },
  diagnostics: {
    eyebrow: "status / 路径 / 日志",
    title: "诊断",
    description: "看原始 stdout / stderr / exit code 与环境路径。",
  },
};

const BACKGROUND_REFRESH_INTERVAL_MS = 60_000;
const STARTUP_REFRESH_DELAY_MS = 2_500;
const DIAGNOSTICS_STATUS_INTERVAL_MS = 30_000;
const MIN_MANUAL_REFRESH_BUSY_MS = 800;
const AUTO_SWITCH_RETRY_COOLDOWN_MS = 5 * 60_000;

type AccountItem = AppSnapshotDto["registry"]["accounts"][number];

type AppAction =
  | { kind: "switch"; query: string }
  | { kind: "remove"; query: string }
  | { kind: "setAlias"; accountKey: string; alias: string }
  | { kind: "importFile"; path: string; alias?: string }
  | { kind: "importDirectory"; path: string }
  | { kind: "importCpa"; path?: string; alias?: string }
  | { kind: "rebuildRegistry"; path?: string }
  | { kind: "autoSwitch"; enabled: boolean }
  | { kind: "usageApi"; enabled: boolean }
  | { kind: "verifyAccount"; accountKey: string }
  | { kind: "launchLogin"; deviceAuth: boolean }
  | { kind: "openPath"; target: string };

type ActionResult = MutationResultDto | CommandExecutionDto | AccountVerificationDto;

function isMutationResult(value: ActionResult): value is MutationResultDto {
  return "registry" in value;
}

function commandFromResult(result: ActionResult) {
  return isMutationResult(result) ? result.command : result;
}

function isAccountVerificationResult(
  value: ActionResult,
): value is AccountVerificationDto {
  return "label" in value && "detail" in value && "switchedBack" in value;
}

function pickQuery(account: AccountItem) {
  return account.accountKey;
}

function suggestedAlias(account: AccountItem) {
  const space = account.accountName?.trim();
  const plan = account.plan?.trim().toLowerCase();
  if (space) {
    return space;
  }
  if (plan) {
    return `${account.email.split("@")[0]}-${plan}`;
  }
  return account.email.split("@")[0];
}

function normalizeOptional(value: string | undefined) {
  const trimmed = value?.trim();
  return trimmed ? trimmed : undefined;
}

function normalizeDialogSelection(value: string | string[] | null) {
  if (Array.isArray(value)) {
    return value[0] ?? null;
  }
  return value;
}

function buildActionError(command: CommandExecutionDto) {
  const output = command.stderr || command.stdout || "命令执行失败";
  if (
    command.category === "config-auto" &&
    output.includes("Register-ScheduledTask") &&
    output.includes("Access is denied")
  ) {
    return [
      "自动切换后台服务启动失败：Windows 计划任务注册被拒绝。",
      "需要用管理员权限运行 codex-auth config auto enable，或升级/修复 codex-auth 的计划任务安装权限。",
      output,
    ].join("\n\n");
  }
  return output;
}

function formatModeLabel(value: string | null | undefined) {
  return value === "api" ? "API" : "本地";
}

function formatFreshnessLabel(value: string | null | undefined) {
  if (value === "fresh") {
    return "最新";
  }
  if (value === "stale") {
    return "滞后";
  }
  return "缺失";
}

function isWorkspaceAccount(plan: string | null | undefined) {
  const normalized = plan?.trim().toLowerCase();
  return (
    normalized === "team" ||
    normalized === "business" ||
    normalized === "enterprise"
  );
}

function getAccountTypeLabel(account: { plan: string }) {
  return isWorkspaceAccount(account.plan) ? "空间" : "个人";
}

function buildAccountNameByChatgptAccountId(accounts: AccountItem[]) {
  const names = new Map<string, string>();
  for (const account of accounts) {
    const accountId = account.chatgptAccountId?.trim();
    const accountName = account.accountName?.trim();
    if (accountId && accountName && !names.has(accountId)) {
      names.set(accountId, accountName);
    }
  }
  return names;
}

function getAccountSpaceLabel(
  account: {
    plan: string;
    accountName: string | null;
  },
  fallbackAccountName?: string,
) {
  const value = account.accountName?.trim() || fallbackAccountName?.trim();
  if (value) {
    return value;
  }
  return isWorkspaceAccount(account.plan) ? "未获取" : "个人账号";
}

function getAccountDisplayLabel(account: AccountItem | null | undefined) {
  if (!account) {
    return "无激活账号";
  }
  const alias = account.alias?.trim();
  const space = account.accountName?.trim();
  if (alias) {
    return `${alias}（${account.email}）`;
  }
  if (space) {
    return `${account.email} / ${space}`;
  }
  return account.email;
}

function isAccountInvalid(account: {
  authStatus: string;
  authStatusCode: number | null;
}) {
  return account.authStatus === "invalid";
}

function isPaidPlan(plan: string | null | undefined) {
  const normalized = plan?.trim().toLowerCase();
  return (
    normalized === "plus" ||
    normalized === "pro" ||
    normalized === "prolite" ||
    normalized === "pro lite" ||
    isWorkspaceAccount(normalized)
  );
}

function isIsoExpired(value: string | null | undefined) {
  if (!value) {
    return false;
  }
  const timestamp = new Date(value).getTime();
  return Number.isFinite(timestamp) && timestamp <= Date.now();
}

function isSubscriptionExpired(account: {
  plan: string;
  subscriptionActiveUntil: string | null;
}) {
  return isPaidPlan(account.plan) && isIsoExpired(account.subscriptionActiveUntil);
}

function isUsageDepleted(usage: AccountItem["primaryUsage"] | null) {
  return usage?.remainingPercent !== null && usage?.remainingPercent !== undefined
    ? usage.remainingPercent <= 0
    : false;
}

function hasNoUsageCredits(account: AccountItem) {
  return (
    account.usageCreditsUnlimited === false &&
    account.usageCreditsHasCredits === false &&
    (account.usageCreditsBalance === "0" || account.usageCreditsBalance === null)
  );
}

function isPrimaryUsageExhausted(account: AccountItem) {
  const remaining = account.primaryUsage?.remainingPercent;
  if (remaining === null || remaining === undefined) {
    return false;
  }
  if (remaining <= 0) {
    return true;
  }
  return remaining <= 1 && isPaidPlan(account.plan) && hasNoUsageCredits(account);
}

function isAccountDisabledByVerification(account: {
  verificationState: string | null;
}) {
  return account.verificationState === "suspected_disabled";
}

function isUsageUnauthorized(account: { authStatus: string }) {
  return account.authStatus === "usage_unauthorized";
}

function hasKnownRemainingUsage(account: AccountItem) {
  return (
    (account.primaryUsage?.remainingPercent ?? null) !== null ||
    (account.weeklyUsage?.remainingPercent ?? null) !== null
  );
}

function isKnownUsableAccount(account: AccountItem) {
  if (
    isAccountInvalid(account) ||
    isAccountDisabledByVerification(account) ||
    isUsageUnauthorized(account) ||
    isSubscriptionExpired(account)
  ) {
    return false;
  }
  if (
    isPrimaryUsageExhausted(account) ||
    isUsageDepleted(account.weeklyUsage)
  ) {
    return false;
  }
  return hasKnownRemainingUsage(account);
}

function shouldAutoVerifyAccount(account: AccountItem) {
  return (
    account.authStatus === "usage_unauthorized" &&
    account.verificationCheckedAtMs === null
  );
}

function shouldAutoSwitchAccount(account: AccountItem | null | undefined) {
  if (!account) {
    return false;
  }
  return (
    isAccountInvalid(account) ||
    isAccountDisabledByVerification(account) ||
    isUsageUnauthorized(account) ||
    isSubscriptionExpired(account) ||
    isPrimaryUsageExhausted(account) ||
    isUsageDepleted(account.weeklyUsage)
  );
}

function accountSwitchScore(account: AccountItem) {
  const remainingValues = [
    account.primaryUsage?.remainingPercent,
    account.weeklyUsage?.remainingPercent,
  ].filter((value): value is number => value !== null && value !== undefined);
  const quotaScore = remainingValues.length ? Math.min(...remainingValues) : 0;
  const lastUsed = getAccountRecentActivityMs(account) ?? 0;
  return { quotaScore, lastUsed };
}

function getAccountRecentActivityMs(account: AccountItem) {
  const timestamps = [account.lastUsedAtMs, account.lastUsageAtMs].filter(
    (value): value is number => typeof value === "number" && Number.isFinite(value),
  );
  return timestamps.length ? Math.max(...timestamps) : null;
}

function pickGuiAutoSwitchTarget(accounts: AccountItem[]) {
  const active = accounts.find((account) => account.active) ?? null;
  if (!shouldAutoSwitchAccount(active)) {
    return null;
  }

  const candidates = accounts
    .filter((account) => !account.active && isKnownUsableAccount(account))
    .sort((left, right) => {
      const leftScore = accountSwitchScore(left);
      const rightScore = accountSwitchScore(right);
      return (
        rightScore.quotaScore - leftScore.quotaScore ||
        leftScore.lastUsed - rightScore.lastUsed ||
        getAccountDisplayLabel(left).localeCompare(getAccountDisplayLabel(right))
      );
    });

  return candidates[0] ?? null;
}

function getAccountStatusLabel(account: {
  plan: string;
  subscriptionActiveUntil: string | null;
  active: boolean;
  authStatus: string;
  authStatusCode: number | null;
  verificationState: string | null;
}) {
  if (isSubscriptionExpired(account)) {
    return "到期";
  }
  if (isAccountInvalid(account)) {
    if (isWorkspaceAccount(account.plan)) {
      return "停用";
    }
    return "失效";
  }
  if (account.authStatus === "usage_unauthorized") {
    return "不可用";
  }
  if (isAccountDisabledByVerification(account)) {
    return isWorkspaceAccount(account.plan) ? "停用" : "不可切换";
  }
  if (account.authStatus === "warning") {
    return "待验证";
  }
  return account.active ? "激活" : "待命";
}

function getAccountStatusVariant(account: {
  plan: string;
  subscriptionActiveUntil: string | null;
  active: boolean;
  authStatus: string;
  authStatusCode: number | null;
  verificationState: string | null;
}) {
  if (
    isSubscriptionExpired(account) ||
    isAccountInvalid(account) ||
    isAccountDisabledByVerification(account) ||
    isUsageUnauthorized(account)
  ) {
    return "destructive" as const;
  }
  return account.active ? ("default" as const) : ("outline" as const);
}

function commandCategoryLabel(category: string) {
  const labels: Record<string, string> = {
    status: "状态查询",
    switch: "切换账号",
    remove: "删除账号",
    "set-alias": "设置别名",
    "import-file": "导入单文件",
    "import-directory": "导入目录",
    "import-cpa": "导入 CPA",
    "rebuild-registry": "重建 registry",
    "refresh-registry": "刷新账号快照",
    "config-auto": "切换自动切换",
    "config-api": "切换 API 模式",
    "launch-login": "启动登录",
    "open-path": "打开路径",
  };
  return labels[category] ?? category;
}

function logState(log: CommandExecutionDto) {
  if (log.timedOut) {
    return {
      label: "超时",
      variant: "destructive" as const,
    };
  }

  if (log.success) {
    return {
      label: "成功",
      variant: "secondary" as const,
    };
  }

  return {
    label: "失败",
    variant: "destructive" as const,
  };
}

function warningVariant(message: string) {
  const normalized = message.toLowerCase();
  if (normalized.includes("invalid") || normalized.includes("失效")) {
    return "destructive" as const;
  }
  return "default" as const;
}

function filterAccounts(accounts: AccountItem[], search: string) {
  const query = search.trim().toLowerCase();
  if (!query) {
    return accounts;
  }

  return accounts.filter((account) => {
    const pool = [
      account.email,
      account.alias,
      account.accountName ?? "",
      account.plan,
    ]
      .join(" ")
      .toLowerCase();
    return pool.includes(query);
  });
}

export default function App() {
  const {
    initialized,
    initialize,
    page,
    setPage,
    accountsSearch,
    setAccountsSearch,
  } = useUiStore();
  const [manualRefreshPending, setManualRefreshPending] = useState(false);
  const [backgroundRefreshPending, setBackgroundRefreshPending] = useState(false);
  const backgroundRefreshInFlightRef = useRef(false);
  const bootstrapRefreshDoneRef = useRef(false);
  const lastActiveAccountRef = useRef<
    { key: string | null; label: string } | undefined
  >(undefined);
  const suppressNextActiveChangeToastRef = useRef(false);
  const lastGuiAutoSwitchAttemptRef = useRef<
    { key: string; attemptedAtMs: number } | undefined
  >(undefined);
  const autoVerificationInFlightRef = useRef(false);
  const deferredAccountsSearch = useDeferredValue(accountsSearch);

  useEffect(() => {
    void initialize();
  }, [initialize]);

  useRegistryEvents();

  const snapshotQuery = useQuery({
    queryKey: ["appSnapshot"],
    queryFn: api.getAppSnapshot,
    enabled: initialized,
    staleTime: 5_000,
    refetchOnWindowFocus: false,
  });

  const statusQuery = useQuery({
    queryKey: ["status"],
    queryFn: api.runCodexAuthStatus,
    enabled: initialized && page === "diagnostics",
    staleTime: 0,
    refetchOnWindowFocus: false,
    refetchInterval:
      initialized && page === "diagnostics"
        ? DIAGNOSTICS_STATUS_INTERVAL_MS
        : false,
  });

  useEffect(() => {
    const account = snapshotQuery.data?.dashboard.activeAccount ?? null;
    const current = {
      key: account?.accountKey ?? null,
      label: getAccountDisplayLabel(account),
    };
    const previous = lastActiveAccountRef.current;

    if (!previous) {
      lastActiveAccountRef.current = current;
      return;
    }

    if (previous.key !== current.key) {
      if (suppressNextActiveChangeToastRef.current) {
        suppressNextActiveChangeToastRef.current = false;
      } else {
        toast.info("当前账号已变化", {
          description: `${previous.label} -> ${current.label}`,
        });
        void logUiEvent("active-account-changed", {
          from: previous,
          to: current,
        });
      }
      lastActiveAccountRef.current = current;
    } else if (previous.label !== current.label) {
      lastActiveAccountRef.current = current;
    }
  }, [snapshotQuery.data?.dashboard.activeAccount]);

  const actionMutation = useMutation<ActionResult, Error, AppAction>({
    mutationFn: async (action) => {
      switch (action.kind) {
        case "switch":
          return api.switchAccount(action.query);
        case "remove":
          return api.removeAccount(action.query);
        case "setAlias":
          return api.setAccountAlias(action.accountKey, action.alias);
        case "importFile":
          return api.importAuthFile(action.path, action.alias);
        case "importDirectory":
          return api.importAuthDirectory(action.path);
        case "importCpa":
          return api.importCpa(action.path, action.alias);
        case "rebuildRegistry":
          return api.rebuildRegistry(action.path);
        case "autoSwitch":
          return api.setAutoSwitch(action.enabled);
        case "usageApi":
          return api.setUsageApiMode(action.enabled);
        case "verifyAccount":
          return api.verifyAccountState(action.accountKey);
        case "launchLogin":
          return api.launchLogin(action.deviceAuth);
        case "openPath":
          return api.openDiagnosticPath(action.target);
      }
    },
  });

  const pendingAction = actionMutation.isPending ? actionMutation.variables : null;
  const busyKind = pendingAction?.kind ?? null;
  const refreshPending = manualRefreshPending || snapshotQuery.isFetching;

  async function logUiEvent(name: string, detail?: unknown) {
    try {
      await api.recordUiEvent(name, detail);
    } catch (error) {
      console.warn("failed to record UI event", name, error);
    }
  }

  async function refetchSnapshotAndStatus(refreshStatus = false) {
    await snapshotQuery.refetch();
    if (refreshStatus && page === "diagnostics") {
      await statusQuery.refetch();
    }
  }

  async function refreshRegistryState(options?: {
    toastOnFailure?: boolean;
    refreshStatus?: boolean;
    immediateLocal?: boolean;
  }) {
    if (options?.immediateLocal) {
      const localStartedAt = performance.now();
      const localResult = await api.getLocalRegistrySnapshot();
      await refetchSnapshotAndStatus(Boolean(options?.refreshStatus));
      await logUiEvent("local-refresh-result", {
        success: localResult.command.success,
        commandDurationMs: localResult.command.durationMs,
        totalDurationMs: Math.round(performance.now() - localStartedAt),
        accountCount: localResult.registry.accounts.length,
      });
    }

    const result = await api.refreshRegistrySnapshot();

    if (!result.command.success && options?.toastOnFailure) {
      toast.error("刷新失败", {
        description: buildActionError(result.command),
      });
    }

    await refetchSnapshotAndStatus(Boolean(options?.refreshStatus));
    return result;
  }

  async function runAction(
    action: AppAction,
    successMessage: string,
    options?: {
      restartHint?: boolean;
      refreshSnapshot?: boolean;
      refreshStatus?: boolean;
      silentSuccess?: boolean;
    },
  ) {
    await logUiEvent("action-start", action);
    let result: ActionResult;
    try {
      result = await actionMutation.mutateAsync(action);
    } catch (error) {
      await logUiEvent("action-error", {
        action,
        message: error instanceof Error ? error.message : String(error),
      });
      throw error;
    }
    const command = commandFromResult(result);
    await logUiEvent("action-result", {
      action,
      success: command.success,
      category: command.category,
      displayCommand: command.displayCommand,
      exitCode: command.exitCode,
      timedOut: command.timedOut,
      stderr: command.stderr,
      stdout: command.stdout,
    });

    if (!command.success) {
      toast.error(`${successMessage}失败`, {
        description: buildActionError(command),
      });
      return result;
    }

    if (!options?.silentSuccess) {
      toast.success(successMessage, {
        description: command.stdout || command.displayCommand,
      });
    }

    if (options?.restartHint) {
      toast.info("切换成功后，请重启 Codex CLI / Codex App");
    }

    const shouldRefreshSnapshot =
      options?.refreshSnapshot ?? isMutationResult(result);
    if (shouldRefreshSnapshot) {
      await refetchSnapshotAndStatus(Boolean(options?.refreshStatus));
    } else if (options?.refreshStatus && page === "diagnostics") {
      await statusQuery.refetch();
    }

    return result;
  }

  async function handleSwitchAccount(query: string) {
    await logUiEvent("switch-click", { query });
    suppressNextActiveChangeToastRef.current = true;
    const result = await runAction(
      { kind: "switch", query },
      "账号已切换",
      {
        refreshStatus: true,
        restartHint: true,
      },
    );
    if (!commandFromResult(result).success) {
      suppressNextActiveChangeToastRef.current = false;
    }
  }

  async function handleVerifyAccount(account: AccountItem) {
    await logUiEvent("verify-account-click", {
      email: account.email,
      accountKey: account.accountKey,
      accountName: account.accountName,
      authStatus: account.authStatus,
      authStatusCode: account.authStatusCode,
    });
    const result = await runAction(
      { kind: "verifyAccount", accountKey: account.accountKey },
      "状态验证完成",
      {
        refreshStatus: true,
        restartHint: true,
      },
    );
    if (isAccountVerificationResult(result)) {
      toast.info(result.label, {
        description: result.detail,
      });
    }
  }

  async function handleRemoveAccount(account: AccountItem) {
    const accepted = await confirm(`确认删除账号 ${account.email} 吗？`, {
      title: "删除账号",
      kind: "warning",
    });

    if (!accepted) {
      return;
    }

    await runAction(
      { kind: "remove", query: pickQuery(account) },
      "账号已删除",
      {
        refreshStatus: true,
      },
    );
  }

  async function handleSetAlias(account: AccountItem) {
    const alias = normalizeOptional(
      window.prompt("输入别名", account.alias || suggestedAlias(account)) ?? undefined,
    );

    await logUiEvent("alias-click", {
      email: account.email,
      accountKey: account.accountKey,
      alias,
    });

    if (!alias) {
      await logUiEvent("alias-cancel", {
        accountKey: account.accountKey,
      });
      return;
    }

    await runAction(
      { kind: "setAlias", accountKey: account.accountKey, alias },
      "别名已设置",
      {
        refreshStatus: true,
      },
    );
  }

  async function handleImportFile(path: string, alias?: string) {
    await runAction(
      { kind: "importFile", path, alias },
      "单文件导入完成",
      {
        refreshStatus: true,
      },
    );
  }

  async function handleImportDirectory(path: string) {
    await runAction(
      { kind: "importDirectory", path },
      "目录导入完成",
      {
        refreshStatus: true,
      },
    );
  }

  async function handleImportCpa(path?: string, alias?: string) {
    await runAction(
      { kind: "importCpa", path, alias },
      "CPA 导入完成",
      {
        refreshStatus: true,
      },
    );
  }

  async function handleRebuildRegistry(path?: string) {
    const accepted = await confirm("确认 purge 并重建 registry 吗？", {
      title: "重建 registry",
      kind: "warning",
    });

    if (!accepted) {
      return;
    }

    await runAction(
      { kind: "rebuildRegistry", path },
      "registry 已重建",
      {
        refreshStatus: true,
      },
    );
  }

  async function handleToggleAutoSwitch(enabled: boolean) {
    const result = await runAction(
      { kind: "autoSwitch", enabled },
      enabled ? "已开启自动切换" : "已关闭自动切换",
      {
        refreshStatus: true,
      },
    );
    if (enabled && isMutationResult(result) && result.command.success) {
      await maybeRunGuiAutoSwitch(result.registry);
    }
  }

  async function handleToggleUsageApi(enabled: boolean) {
    if (enabled) {
      const accepted = await confirm(
        "开启 API 模式后，codex-auth 会读取 usage 与 team 信息。仅用于 GUI 展示，不会保存 token 正文。确认继续？",
        {
          title: "开启 API 模式",
          kind: "warning",
        },
      );

      if (!accepted) {
        return;
      }
    }

    await runAction(
      { kind: "usageApi", enabled },
      enabled ? "已开启 API 模式" : "已关闭 API 模式",
      {
        refreshStatus: true,
        silentSuccess: true,
      },
    );
  }

  async function handleLaunchLogin(deviceAuth: boolean) {
    await logUiEvent("launch-login-click", { deviceAuth });
    await runAction(
      { kind: "launchLogin", deviceAuth },
      deviceAuth ? "设备码登录已启动" : "网页登录已启动",
      {
        silentSuccess: true,
        refreshSnapshot: false,
      },
    );

    toast.info(
      deviceAuth
        ? "设备码窗口已打开。完成登录后回账号页切换。"
        : "登录窗口已打开，并会自动打开授权网页。完成后稍等自动刷新或手动点刷新。",
    );
  }

  async function handleReloginAccount(account: AccountItem) {
    await logUiEvent("relogin-click", {
      email: account.email,
      accountKey: account.accountKey,
      authStatus: account.authStatus,
      authStatusCode: account.authStatusCode,
    });
    await handleLaunchLogin(false);
  }

  async function handleOpenPath(target: string) {
    await runAction(
      { kind: "openPath", target },
      "目录已打开",
      {
        silentSuccess: true,
        refreshSnapshot: false,
      },
    );
  }

  async function handleHeaderRefresh() {
    await logUiEvent("header-refresh-click", {
      manualRefreshPending,
      backgroundRefreshPending,
      snapshotFetching: snapshotQuery.isFetching,
      backgroundRefreshInFlight: backgroundRefreshInFlightRef.current,
      page,
    });

    if (
      manualRefreshPending ||
      backgroundRefreshInFlightRef.current ||
      snapshotQuery.isFetching
    ) {
      await logUiEvent("header-refresh-skip", {
        manualRefreshPending,
        backgroundRefreshPending,
        snapshotFetching: snapshotQuery.isFetching,
        backgroundRefreshInFlight: backgroundRefreshInFlightRef.current,
      });
      return;
    }

    setManualRefreshPending(true);
    const startedAt = performance.now();
    try {
      const result = await refreshRegistryState({
        toastOnFailure: true,
        refreshStatus: true,
        immediateLocal: true,
      });
      await logUiEvent("header-refresh-result", {
        success: result.command.success,
        exitCode: result.command.exitCode,
        timedOut: result.command.timedOut,
        displayCommand: result.command.displayCommand,
        commandDurationMs: result.command.durationMs,
        totalDurationMs: Math.round(performance.now() - startedAt),
        accountCount: result.registry.accounts.length,
        stderr: result.command.stderr,
      });
    } finally {
      const remainingMs = MIN_MANUAL_REFRESH_BUSY_MS - (performance.now() - startedAt);
      if (remainingMs > 0) {
        await new Promise((resolve) => window.setTimeout(resolve, remainingMs));
      }
      setManualRefreshPending(false);
      await logUiEvent("header-refresh-done", {
        durationMs: Math.round(performance.now() - startedAt),
      });
    }
  }

  async function maybeRunGuiAutoSwitch(registry: AppSnapshotDto["registry"]) {
    if (!registry.autoSwitchEnabled) {
      return;
    }

    const target = pickGuiAutoSwitchTarget(registry.accounts);
    if (!target) {
      return;
    }

    const now = Date.now();
    const lastAttempt = lastGuiAutoSwitchAttemptRef.current;
    if (
      lastAttempt?.key === target.accountKey &&
      now - lastAttempt.attemptedAtMs < AUTO_SWITCH_RETRY_COOLDOWN_MS
    ) {
      await logUiEvent("gui-auto-switch-skip", {
        reason: "cooldown",
        target: getAccountDisplayLabel(target),
      });
      return;
    }

    const active = registry.accounts.find((account) => account.active) ?? null;
    lastGuiAutoSwitchAttemptRef.current = {
      key: target.accountKey,
      attemptedAtMs: now,
    };
    await logUiEvent("gui-auto-switch-start", {
      from: active ? getAccountDisplayLabel(active) : null,
      to: getAccountDisplayLabel(target),
      targetAccountKey: target.accountKey,
    });

    const result = await api.switchAccount(target.accountKey);
    await logUiEvent("gui-auto-switch-result", {
      success: result.command.success,
      exitCode: result.command.exitCode,
      stderr: result.command.stderr,
      stdout: result.command.stdout,
    });

    if (result.command.success) {
      toast.success("自动切换已执行", {
        description: `${getAccountDisplayLabel(active)} -> ${getAccountDisplayLabel(target)}`,
      });
      await refetchSnapshotAndStatus(page === "diagnostics");
    } else {
      toast.error("自动切换失败", {
        description: buildActionError(result.command),
      });
    }
  }

  async function maybeRunAutoVerification(registry: AppSnapshotDto["registry"]) {
    if (autoVerificationInFlightRef.current || actionMutation.isPending) {
      return;
    }

    const target = registry.accounts.find(shouldAutoVerifyAccount);
    if (!target) {
      return;
    }

    autoVerificationInFlightRef.current = true;
    await logUiEvent("auto-verification-start", {
      target: getAccountDisplayLabel(target),
      targetAccountKey: target.accountKey,
      authStatus: target.authStatus,
      authStatusCode: target.authStatusCode,
    });

    try {
      const result = await api.verifyAccountState(target.accountKey);
      await logUiEvent("auto-verification-result", {
        target: getAccountDisplayLabel(target),
        success: result.command.success,
        state: result.state,
        label: result.label,
        detail: result.detail,
        switchedBack: result.switchedBack,
      });
      await refetchSnapshotAndStatus(page === "diagnostics");
    } catch (error) {
      await logUiEvent("auto-verification-error", {
        target: getAccountDisplayLabel(target),
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      autoVerificationInFlightRef.current = false;
    }
  }

  const runBackgroundRefresh = useEffectEvent(async () => {
    if (!initialized || !snapshotQuery.data) {
      await logUiEvent("background-refresh-skip", {
        reason: "not-ready",
        initialized,
        hasSnapshot: Boolean(snapshotQuery.data),
      });
      return;
    }

    if (document.visibilityState !== "visible") {
      await logUiEvent("background-refresh-skip", {
        reason: "hidden",
        visibilityState: document.visibilityState,
      });
      return;
    }

    if (
      backgroundRefreshInFlightRef.current ||
      manualRefreshPending ||
      snapshotQuery.isFetching ||
      actionMutation.isPending
    ) {
      await logUiEvent("background-refresh-skip", {
        reason: "busy",
        backgroundRefreshInFlight: backgroundRefreshInFlightRef.current,
        manualRefreshPending,
        snapshotFetching: snapshotQuery.isFetching,
        actionPending: actionMutation.isPending,
      });
      return;
    }

    void api
      .getLocalRegistrySnapshot()
      .then(async (localResult) => {
        await refetchSnapshotAndStatus(page === "diagnostics");
        await logUiEvent("background-local-refresh-result", {
          success: localResult.command.success,
          commandDurationMs: localResult.command.durationMs,
          accountCount: localResult.registry.accounts.length,
        });
      })
      .catch((error) => {
        void logUiEvent("background-local-refresh-error", {
          message: error instanceof Error ? error.message : String(error),
        });
      });

    backgroundRefreshInFlightRef.current = true;
    setBackgroundRefreshPending(true);
    const startedAt = performance.now();
    await logUiEvent("background-refresh-start", { page });

    try {
      const result = await refreshRegistryState({
        refreshStatus: page === "diagnostics",
      });
      await logUiEvent("background-refresh-result", {
        success: result.command.success,
        exitCode: result.command.exitCode,
        timedOut: result.command.timedOut,
        displayCommand: result.command.displayCommand,
        commandDurationMs: result.command.durationMs,
        totalDurationMs: Math.round(performance.now() - startedAt),
        accountCount: result.registry.accounts.length,
        stderr: result.command.stderr,
      });
      if (result.command.success) {
        await maybeRunAutoVerification(result.registry);
        await maybeRunGuiAutoSwitch(result.registry);
      }
    } finally {
      backgroundRefreshInFlightRef.current = false;
      setBackgroundRefreshPending(false);
      await logUiEvent("background-refresh-done", {
        durationMs: Math.round(performance.now() - startedAt),
      });
    }
  });

  useEffect(() => {
    if (!initialized || !snapshotQuery.data || bootstrapRefreshDoneRef.current) {
      return;
    }

    bootstrapRefreshDoneRef.current = true;
    const timer = window.setTimeout(() => {
      void runBackgroundRefresh();
    }, STARTUP_REFRESH_DELAY_MS);

    return () => {
      window.clearTimeout(timer);
    };
  }, [initialized, snapshotQuery.data, runBackgroundRefresh]);

  useEffect(() => {
    if (!initialized) {
      return;
    }

    const runVisibleRefresh = () => {
      if (document.visibilityState === "visible") {
        void runBackgroundRefresh();
      }
    };

    window.addEventListener("focus", runVisibleRefresh);
    document.addEventListener("visibilitychange", runVisibleRefresh);

    return () => {
      window.removeEventListener("focus", runVisibleRefresh);
      document.removeEventListener("visibilitychange", runVisibleRefresh);
    };
  }, [initialized, runBackgroundRefresh]);

  useEffect(() => {
    if (!initialized) {
      return;
    }

    const timer = window.setInterval(() => {
      void runBackgroundRefresh();
    }, BACKGROUND_REFRESH_INTERVAL_MS);

    return () => {
      window.clearInterval(timer);
    };
  }, [initialized, runBackgroundRefresh]);

  if (!initialized) {
    return <BootSkeleton />;
  }

  if (snapshotQuery.isPending && !snapshotQuery.data) {
    return <BootSkeleton />;
  }

  if (snapshotQuery.isError || !snapshotQuery.data) {
    return (
      <TooltipProvider>
        <div className="min-h-svh bg-background p-6">
          <Card className="mx-auto max-w-xl">
            <CardHeader>
              <CardTitle>加载失败</CardTitle>
              <CardDescription>无法读取本机 codex-auth 快照。</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <Alert variant="destructive">
                <AlertCircleIcon />
                <AlertTitle>错误</AlertTitle>
                <AlertDescription>
                  {(snapshotQuery.error as Error | undefined)?.message ??
                    "未知错误"}
                </AlertDescription>
              </Alert>
              <Button onClick={() => void snapshotQuery.refetch()}>
                <RefreshCcwIcon className="mr-1" />
                重试
              </Button>
            </CardContent>
          </Card>
          <Toaster richColors />
        </div>
      </TooltipProvider>
    );
  }

  const snapshot = snapshotQuery.data;
  const activeAccount = snapshot.dashboard.activeAccount;
  const accountNameByChatgptAccountId = buildAccountNameByChatgptAccountId(
    snapshot.registry.accounts,
  );
  const filteredAccounts = filterAccounts(
    snapshot.registry.accounts,
    deferredAccountsSearch,
  );

  return (
    <TooltipProvider>
      <SidebarProvider defaultOpen>
        <div className="codex-auth-shell">
          <Sidebar
            collapsible="icon"
            variant="sidebar"
            className="border-r border-sidebar-border/70"
          >
            <SidebarHeader className="gap-4 p-4">
              <div className="rounded-3xl border border-sidebar-border/80 bg-sidebar px-3 py-4 shadow-sm">
                <div className="text-[0.68rem] font-medium uppercase tracking-[0.28em] text-muted-foreground">
                  Windows 11 本地
                </div>
                <div className="mt-2 text-[1.1rem] font-semibold">
                  Codex Auth GUI
                </div>
                <div className="mt-2 text-sm leading-7 text-muted-foreground">
                  本地图形界面。只做 codex-auth 操作台，不重写认证逻辑。
                </div>
              </div>
            </SidebarHeader>

            <SidebarContent>
              <SidebarGroup>
                <SidebarGroupLabel>工作区</SidebarGroupLabel>
                <SidebarGroupContent>
                  <SidebarMenu>
                    {NAV_ITEMS.map((item) => {
                      const Icon = item.icon;
                      return (
                        <SidebarMenuItem key={item.key}>
                          <SidebarMenuButton
                            tooltip={item.label}
                            isActive={page === item.key}
                            onClick={() => setPage(item.key)}
                          >
                            <Icon />
                            <span>{item.label}</span>
                          </SidebarMenuButton>
                        </SidebarMenuItem>
                      );
                    })}
                  </SidebarMenu>
                </SidebarGroupContent>
              </SidebarGroup>

              <SidebarGroup>
                <SidebarGroupLabel>状态</SidebarGroupLabel>
                <SidebarGroupContent className="space-y-3 px-2">
                  <SidebarStatCard
                    label="模式"
                    value={formatModeLabel(snapshot.registry.usageMode)}
                  />
                  <SidebarStatCard
                    label="自动切换"
                    value={snapshot.registry.autoSwitchEnabled ? "已开启" : "已关闭"}
                  />
                  <SidebarStatCard
                    label="账号数"
                    value={String(snapshot.registry.accounts.length)}
                  />
                </SidebarGroupContent>
              </SidebarGroup>
            </SidebarContent>

            <SidebarFooter className="p-4">
              <Card size="sm" className="rounded-3xl">
                <CardHeader className="gap-2">
                  <CardDescription>当前账号</CardDescription>
                  <CardTitle className="break-all text-sm leading-6">
                    {activeAccount?.email ?? "暂无激活账号"}
                  </CardTitle>
                </CardHeader>
                <CardContent className="flex flex-wrap gap-2">
                  <Badge variant="secondary">
                    {activeAccount ? getAccountTypeLabel(activeAccount) : "空闲"}
                  </Badge>
                  <Badge variant="outline">
                    {formatModeLabel(snapshot.registry.usageMode)}
                  </Badge>
                  {activeAccount && isAccountInvalid(activeAccount) ? (
                    <Badge variant="destructive">失效</Badge>
                  ) : null}
                </CardContent>
              </Card>
            </SidebarFooter>
          </Sidebar>

          <SidebarInset>
            <header className="sticky top-0 z-20 border-b border-border/70 bg-background/95 px-4 py-4 backdrop-blur supports-[backdrop-filter]:bg-background/80 md:px-6">
              <div className="flex min-w-0 items-start justify-between gap-4">
                <div className="flex min-w-0 items-start gap-3">
                  <SidebarTrigger className="mt-0.5 md:hidden" />
                  <div className="min-w-0">
                    <div className="text-xs uppercase tracking-[0.24em] text-muted-foreground">
                      {PAGE_META[page].eyebrow}
                    </div>
                    <div className="mt-1 flex flex-wrap items-center gap-2">
                      <h1 className="text-3xl font-semibold tracking-tight">
                        {PAGE_META[page].title}
                      </h1>
                      <Badge variant="outline">
                        {snapshot.registry.accounts.length} 个账号
                      </Badge>
                      <Badge variant="outline">
                        自动切换
                        {snapshot.registry.autoSwitchEnabled ? "开启" : "关闭"}
                      </Badge>
                    </div>
                    <p className="mt-2 text-sm text-muted-foreground">
                      {PAGE_META[page].description}
                    </p>
                  </div>
                </div>

                <Button
                  variant="outline"
                  onClick={() => void handleHeaderRefresh()}
                  disabled={refreshPending}
                >
                  {refreshPending ? (
                    <Loader2Icon className="mr-1 animate-spin" />
                  ) : (
                    <RefreshCcwIcon className="mr-1" />
                  )}
                  刷新
                </Button>
              </div>
            </header>

            <main className="min-w-0 flex-1 overflow-y-auto px-4 py-6 md:px-6">
              <div className="mx-auto max-w-[1600px] space-y-6">
                <GlobalWarnings warnings={snapshot.dashboard.warnings} />

                {page === "dashboard" ? (
                  <DashboardPage snapshot={snapshot} />
                ) : null}

                {page === "accounts" ? (
                  <AccountsPage
                    accounts={filteredAccounts}
                    accountNameByChatgptAccountId={accountNameByChatgptAccountId}
                    totalAccounts={snapshot.registry.accounts.length}
                    searchValue={accountsSearch}
                    pendingAction={pendingAction}
                    onSearchChange={setAccountsSearch}
                    onSwitch={handleSwitchAccount}
                    onVerify={handleVerifyAccount}
                    onRemove={handleRemoveAccount}
                    onRelogin={handleReloginAccount}
                    onSetAlias={handleSetAlias}
                  />
                ) : null}

                {page === "import" ? (
                  <ImportPage
                    busyKind={busyKind}
                    onImportFile={handleImportFile}
                    onImportDirectory={handleImportDirectory}
                    onImportCpa={handleImportCpa}
                    onRebuildRegistry={handleRebuildRegistry}
                  />
                ) : null}

                {page === "settings" ? (
                  <SettingsPage
                    snapshot={snapshot}
                    busyKind={busyKind}
                    onToggleAutoSwitch={handleToggleAutoSwitch}
                    onToggleUsageApi={handleToggleUsageApi}
                    onLaunchLogin={handleLaunchLogin}
                  />
                ) : null}

                {page === "diagnostics" ? (
                  <DiagnosticsPage
                    snapshot={snapshot}
                    statusLog={statusQuery.data ?? snapshot.diagnostics.latestStatusLog}
                    statusBusy={statusQuery.isFetching}
                    onRunStatus={() => void statusQuery.refetch()}
                    onOpenPath={handleOpenPath}
                  />
                ) : null}
              </div>
            </main>
          </SidebarInset>
          <SidebarRail />
        </div>
        <Toaster richColors closeButton />
      </SidebarProvider>
    </TooltipProvider>
  );
}

function GlobalWarnings({ warnings }: { warnings: string[] }) {
  if (!warnings.length) {
    return null;
  }

  return (
    <div className="space-y-3">
      {warnings.map((warning, index) => (
        <Alert key={`${warning}-${index}`} variant={warningVariant(warning)}>
          <ShieldAlertIcon />
          <AlertTitle>注意</AlertTitle>
          <AlertDescription>{warning}</AlertDescription>
        </Alert>
      ))}
    </div>
  );
}

function DashboardPage({ snapshot }: { snapshot: AppSnapshotDto }) {
  const activeAccount = snapshot.dashboard.activeAccount;
  const accountNameByChatgptAccountId = buildAccountNameByChatgptAccountId(
    snapshot.registry.accounts,
  );
  const activeAccountFallbackName = activeAccount?.chatgptAccountId
    ? accountNameByChatgptAccountId.get(activeAccount.chatgptAccountId)
    : undefined;

  return (
    <div className="space-y-6">
      <div className="grid gap-4 xl:grid-cols-[minmax(0,2fr)_minmax(320px,1fr)]">
        <Card className="rounded-3xl">
          <CardHeader>
            <CardDescription>当前激活账号</CardDescription>
            {activeAccount ? (
              <>
                <div className="flex flex-wrap items-center gap-2">
                  <CardTitle className="break-all text-4xl leading-tight">
                    {activeAccount.email}
                  </CardTitle>
                  <Badge variant="secondary">
                    {fallbackText(activeAccount.plan, "未知套餐")}
                  </Badge>
                  <Badge variant="outline">
                    {getAccountTypeLabel(activeAccount)}
                  </Badge>
                  <Badge variant="outline">
                    {formatModeLabel(snapshot.dashboard.usageMode)}
                  </Badge>
                  <Badge variant={getAccountStatusVariant(activeAccount)}>
                    {getAccountStatusLabel(activeAccount)}
                  </Badge>
                </div>
                <CardDescription className="space-y-1">
                  <div>别名：{fallbackText(activeAccount.alias, "未设置")}</div>
                  <div>
                    所属空间：
                    {getAccountSpaceLabel(activeAccount, activeAccountFallbackName)}
                  </div>
                </CardDescription>
              </>
            ) : (
              <>
                <CardTitle>暂无激活账号</CardTitle>
                <CardDescription>先导入或登录，再去账号页切换。</CardDescription>
              </>
            )}
          </CardHeader>
          <CardContent className="space-y-4">
            {activeAccount ? (
              <>
                {isAccountInvalid(activeAccount) ? (
                  <Alert variant="destructive">
                    <ShieldAlertIcon />
                    <AlertTitle>认证失效</AlertTitle>
                    <AlertDescription>
                      当前激活账号已失效。请重新登录，或切到健康账号。
                    </AlertDescription>
                  </Alert>
                ) : null}

                <div className="grid gap-3 md:grid-cols-3">
                  <MetricCard
                    label="最近活动"
                    value={formatRelative(getAccountRecentActivityMs(activeAccount))}
                    detail={formatTimestamp(getAccountRecentActivityMs(activeAccount))}
                  />
                  <MetricCard
                    label="最近认证检查"
                    value={formatRelative(activeAccount.authCheckedAtMs)}
                    detail={formatTimestamp(activeAccount.authCheckedAtMs)}
                  />
                  <MetricCard
                    label="数据新鲜度"
                    value={formatFreshnessLabel(snapshot.dashboard.dataFreshness)}
                    detail={
                      snapshot.dashboard.dataFreshness === "fresh"
                        ? "当前数据较新。"
                        : snapshot.dashboard.dataFreshness === "stale"
                          ? "本地快照偏旧。"
                          : "尚无可用快照。"
                    }
                  />
                </div>
              </>
            ) : (
              <Empty className="rounded-2xl border border-dashed">
                <EmptyHeader>
                  <EmptyMedia variant="icon">
                    <UserCog2Icon />
                  </EmptyMedia>
                  <EmptyTitle>没有激活账号</EmptyTitle>
                  <EmptyDescription>
                    去“导入”或“设置”新增账号，再到“账号”页切换。
                  </EmptyDescription>
                </EmptyHeader>
              </Empty>
            )}
          </CardContent>
        </Card>

        <div className="grid gap-4">
          <UsageCard
            title="5 小时剩余"
            usage={activeAccount?.primaryUsage ?? null}
          />
          <UsageCard
            title="本周剩余"
            usage={activeAccount?.weeklyUsage ?? null}
          />
        </div>
      </div>

      <Card className="rounded-3xl">
        <CardHeader>
          <CardTitle>环境检查</CardTitle>
          <CardDescription>
            codex-auth / codex / node 路径与版本探测。
          </CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4 lg:grid-cols-3">
          {snapshot.dashboard.envChecks.map((check) => (
            <EnvCheckCard key={check.key} check={check} />
          ))}
        </CardContent>
      </Card>
    </div>
  );
}

function AccountsPage({
  accounts,
  accountNameByChatgptAccountId,
  totalAccounts,
  searchValue,
  pendingAction,
  onSearchChange,
  onSwitch,
  onVerify,
  onRemove,
  onRelogin,
  onSetAlias,
}: {
  accounts: AccountItem[];
  accountNameByChatgptAccountId: Map<string, string>;
  totalAccounts: number;
  searchValue: string;
  pendingAction: AppAction | null;
  onSearchChange: (value: string) => void;
  onSwitch: (query: string) => Promise<void>;
  onVerify: (account: AccountItem) => Promise<void>;
  onRemove: (account: AccountItem) => Promise<void>;
  onRelogin: (account: AccountItem) => Promise<void>;
  onSetAlias: (account: AccountItem) => Promise<void>;
}) {
  return (
    <Card className="rounded-3xl">
      <CardHeader className="gap-4 md:flex-row md:items-start md:justify-between">
        <div className="space-y-1">
          <CardTitle>账号列表</CardTitle>
          <CardDescription>
            读 registry.json。切换 / 删除统一调用 codex-auth switch / remove。
          </CardDescription>
        </div>

        <div className="flex w-full max-w-md items-center gap-2">
          <div className="relative flex-1">
            <SearchIcon className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={searchValue}
              onChange={(event) => onSearchChange(event.target.value)}
              placeholder="按 alias / email / 空间名 搜索"
              className="pl-9"
            />
          </div>
          <Badge variant="outline">{accounts.length} / {totalAccounts}</Badge>
        </div>
      </CardHeader>

      <CardContent>
        {accounts.length ? (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>账号</TableHead>
                <TableHead>类型</TableHead>
                <TableHead>所属空间</TableHead>
                <TableHead>套餐</TableHead>
                <TableHead>5 小时</TableHead>
                <TableHead>本周</TableHead>
                <TableHead>最近活动</TableHead>
                <TableHead>状态</TableHead>
                <TableHead className="text-right">操作</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {accounts.map((account) => {
                const fallbackAccountName = account.chatgptAccountId
                  ? accountNameByChatgptAccountId.get(account.chatgptAccountId)
                  : undefined;
                const switchBusy =
                  pendingAction?.kind === "switch" &&
                  pendingAction.query === account.accountKey;
                const removeBusy =
                  pendingAction?.kind === "remove" &&
                  pendingAction.query === account.accountKey;
                const aliasBusy =
                  pendingAction?.kind === "setAlias" &&
                  pendingAction.accountKey === account.accountKey;
                const verifyBusy =
                  pendingAction?.kind === "verifyAccount" &&
                  pendingAction.accountKey === account.accountKey;
                const reloginBusy =
                  pendingAction?.kind === "launchLogin" && isAccountInvalid(account);

                return (
                  <TableRow key={account.accountKey}>
                    <TableCell className="whitespace-normal align-top">
                      <div className="space-y-1">
                        <div className="break-all font-medium">{account.email}</div>
                        <div className="text-sm text-muted-foreground">
                          别名：{fallbackText(account.alias, "未设置")}
                        </div>
                      </div>
                    </TableCell>
                    <TableCell className="whitespace-normal align-top">
                      <Badge variant="secondary">
                        {getAccountTypeLabel(account)}
                      </Badge>
                    </TableCell>
                    <TableCell className="whitespace-normal align-top">
                      {getAccountSpaceLabel(account, fallbackAccountName)}
                    </TableCell>
                    <TableCell className="whitespace-normal align-top">
                      {fallbackText(account.plan, "未知")}
                    </TableCell>
                    <TableCell className="whitespace-normal align-top">
                      <UsageQuotaCell
                        usage={account.primaryUsage}
                        exhausted={isPrimaryUsageExhausted(account)}
                      />
                    </TableCell>
                    <TableCell className="whitespace-normal align-top">
                      <UsageQuotaCell usage={account.weeklyUsage} />
                    </TableCell>
                    <TableCell className="whitespace-normal align-top">
                      <div>{formatRelative(getAccountRecentActivityMs(account))}</div>
                      <div className="text-sm text-muted-foreground">
                        {formatTimestamp(getAccountRecentActivityMs(account))}
                      </div>
                    </TableCell>
                    <TableCell className="whitespace-normal align-top">
                      <div className="space-y-2">
                        <Badge variant={getAccountStatusVariant(account)}>
                          {getAccountStatusLabel(account)}
                        </Badge>
                        {account.authStatusDetail ? (
                          <div className="text-sm text-muted-foreground">
                            {account.authStatusDetail}
                          </div>
                        ) : null}
                        {account.verificationLabel ? (
                          <div className="text-sm text-muted-foreground">
                            验证：{account.verificationLabel}
                            {account.verificationCheckedAtMs
                              ? ` · ${formatRelative(account.verificationCheckedAtMs)}`
                              : ""}
                          </div>
                        ) : null}
                        {account.verificationDetail ? (
                          <div className="text-xs text-muted-foreground">
                            {account.verificationDetail}
                          </div>
                        ) : null}
                        <AccountLifecycleDetail account={account} />
                      </div>
                    </TableCell>
                    <TableCell className="whitespace-normal align-top">
                      <div className="flex flex-wrap justify-end gap-2">
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={aliasBusy}
                          onClick={() => void onSetAlias(account)}
                        >
                          {aliasBusy ? (
                            <Loader2Icon className="mr-1 animate-spin" />
                          ) : (
                            <PencilIcon className="mr-1" />
                          )}
                          别名
                        </Button>
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={
                            account.active ||
                            isAccountInvalid(account) ||
                            isAccountDisabledByVerification(account) ||
                            isUsageUnauthorized(account) ||
                            switchBusy ||
                            verifyBusy
                          }
                          onClick={() => void onSwitch(pickQuery(account))}
                        >
                          {switchBusy ? (
                            <Loader2Icon className="mr-1 animate-spin" />
                          ) : null}
                          切换
                        </Button>
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={verifyBusy || switchBusy}
                          onClick={() => void onVerify(account)}
                        >
                          {verifyBusy ? (
                            <Loader2Icon className="mr-1 animate-spin" />
                          ) : null}
                          验证
                        </Button>
                        {isAccountInvalid(account) ? (
                          <Button
                            variant="outline"
                            size="sm"
                            disabled={reloginBusy}
                            onClick={() => void onRelogin(account)}
                          >
                            {reloginBusy ? (
                              <Loader2Icon className="mr-1 animate-spin" />
                            ) : null}
                            重新登录
                          </Button>
                        ) : null}
                        <Button
                          variant="destructive"
                          size="sm"
                          disabled={removeBusy}
                          onClick={() => void onRemove(account)}
                        >
                          {removeBusy ? (
                            <Loader2Icon className="mr-1 animate-spin" />
                          ) : (
                            <Trash2Icon className="mr-1" />
                          )}
                          删除
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        ) : (
          <Empty className="rounded-2xl border border-dashed">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <SearchIcon />
              </EmptyMedia>
              <EmptyTitle>没有匹配账号</EmptyTitle>
              <EmptyDescription>
                试试 email、alias，或清空搜索词。
              </EmptyDescription>
            </EmptyHeader>
          </Empty>
        )}
      </CardContent>
    </Card>
  );
}

function UsageQuotaCell({
  usage,
  exhausted = false,
}: {
  usage: AccountItem["primaryUsage"] | null;
  exhausted?: boolean;
}) {
  const resetAtMs = usage?.resetsAtMs ?? null;

  return (
    <div className="space-y-1">
      <div>{exhausted ? "0%" : formatPercent(usage?.remainingPercent)}</div>
      {resetAtMs ? (
        <>
          <div className="text-sm text-muted-foreground">
            恢复：{formatShortTimestamp(resetAtMs)}
          </div>
          <div className="text-xs text-muted-foreground">
            {formatRelative(resetAtMs)}
          </div>
        </>
      ) : (
        <div className="text-sm text-muted-foreground">恢复：暂无</div>
      )}
    </div>
  );
}

function AccountLifecycleDetail({ account }: { account: AccountItem }) {
  const subscriptionExpired = isSubscriptionExpired(account);

  return (
    <div className="space-y-1 text-xs text-muted-foreground">
      {account.subscriptionActiveUntil ? (
        <div className={subscriptionExpired ? "text-destructive" : undefined}>
          套餐：
          {subscriptionExpired ? "已到期，" : ""}
          {formatIsoTimestamp(account.subscriptionActiveUntil)}
        </div>
      ) : null}
      {account.subscriptionLastChecked ? (
        <div>套餐检查：{formatIsoRelative(account.subscriptionLastChecked)}</div>
      ) : null}
      {account.authLastRefresh ? (
        <div>登录凭据刷新：{formatIsoRelative(account.authLastRefresh)}</div>
      ) : null}
      {!account.authHasRefreshToken ? (
        <div className="text-destructive">缺少 refresh token，可能需要重新登录</div>
      ) : null}
    </div>
  );
}

function ImportPage({
  busyKind,
  onImportFile,
  onImportDirectory,
  onImportCpa,
  onRebuildRegistry,
}: {
  busyKind: AppAction["kind"] | null;
  onImportFile: (path: string, alias?: string) => Promise<void>;
  onImportDirectory: (path: string) => Promise<void>;
  onImportCpa: (path?: string, alias?: string) => Promise<void>;
  onRebuildRegistry: (path?: string) => Promise<void>;
}) {
  const importFileForm = useForm<z.infer<typeof importFileSchema>>({
    resolver: zodResolver(importFileSchema),
    defaultValues: {
      path: "",
      alias: "",
    },
  });

  const importDirectoryForm = useForm<z.infer<typeof importDirectorySchema>>({
    resolver: zodResolver(importDirectorySchema),
    defaultValues: {
      path: "",
    },
  });

  const importCpaForm = useForm<z.infer<typeof importCpaSchema>>({
    resolver: zodResolver(importCpaSchema),
    defaultValues: {
      path: "",
      alias: "",
    },
  });

  const rebuildRegistryForm = useForm<z.infer<typeof rebuildRegistrySchema>>({
    resolver: zodResolver(rebuildRegistrySchema),
    defaultValues: {
      path: "",
    },
  });

  async function pickJsonFile(
    setter: (value: string) => void,
  ) {
    const selected = normalizeDialogSelection(
      await open({
        multiple: false,
        directory: false,
        filters: [
          {
            name: "JSON",
            extensions: ["json"],
          },
        ],
      }),
    );

    if (selected) {
      setter(selected);
    }
  }

  async function pickDirectory(setter: (value: string) => void) {
    const selected = normalizeDialogSelection(
      await open({
        multiple: false,
        directory: true,
      }),
    );

    if (selected) {
      setter(selected);
    }
  }

  return (
    <div className="grid gap-4 xl:grid-cols-2">
      <Card className="rounded-3xl">
        <CardHeader>
          <CardTitle>单个账号 JSON</CardTitle>
          <CardDescription>
            导入单个账号凭据文件。不是 registry.json。
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            className="space-y-4"
            onSubmit={importFileForm.handleSubmit(async (values) => {
              await onImportFile(values.path, normalizeOptional(values.alias));
            })}
          >
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="import-file-path">JSON 文件</FieldLabel>
                <FieldDescription>常见是 *.auth.json。</FieldDescription>
                <div className="flex gap-2">
                  <Input
                    id="import-file-path"
                    placeholder="选择单个账号 JSON"
                    {...importFileForm.register("path")}
                  />
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() =>
                      void pickJsonFile((value) =>
                        importFileForm.setValue("path", value, {
                          shouldDirty: true,
                          shouldValidate: true,
                        }),
                      )
                    }
                  >
                    <FolderOpenIcon className="mr-1" />
                    浏览
                  </Button>
                </div>
                <FieldError errors={[importFileForm.formState.errors.path]} />
              </Field>

              <Field>
                <FieldLabel htmlFor="import-file-alias">别名</FieldLabel>
                <FieldDescription>可留空。</FieldDescription>
                <Input
                  id="import-file-alias"
                  placeholder="如：主号 / 团队号"
                  {...importFileForm.register("alias")}
                />
                <FieldError errors={[importFileForm.formState.errors.alias]} />
              </Field>
            </FieldGroup>

            <Button
              type="submit"
              disabled={busyKind === "importFile"}
            >
              {busyKind === "importFile" ? (
                <Loader2Icon className="mr-1 animate-spin" />
              ) : (
                <HardDriveDownloadIcon className="mr-1" />
              )}
              导入单文件
            </Button>
          </form>
        </CardContent>
      </Card>

      <Card className="rounded-3xl">
        <CardHeader>
          <CardTitle>账号目录</CardTitle>
          <CardDescription>
            整目录导入。适合从旧机器迁移 `.codex/accounts`。
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            className="space-y-4"
            onSubmit={importDirectoryForm.handleSubmit(async (values) => {
              await onImportDirectory(values.path);
            })}
          >
            <Field>
              <FieldLabel htmlFor="import-directory-path">目录路径</FieldLabel>
              <FieldDescription>目录里应包含账号 JSON 文件。</FieldDescription>
              <div className="flex gap-2">
                <Input
                  id="import-directory-path"
                  placeholder="选择账号目录"
                  {...importDirectoryForm.register("path")}
                />
                <Button
                  type="button"
                  variant="outline"
                  onClick={() =>
                    void pickDirectory((value) =>
                      importDirectoryForm.setValue("path", value, {
                        shouldDirty: true,
                        shouldValidate: true,
                      }),
                    )
                  }
                >
                  <FolderOpenIcon className="mr-1" />
                  浏览
                </Button>
              </div>
              <FieldError errors={[importDirectoryForm.formState.errors.path]} />
            </Field>

            <Button
              type="submit"
              disabled={busyKind === "importDirectory"}
            >
              {busyKind === "importDirectory" ? (
                <Loader2Icon className="mr-1 animate-spin" />
              ) : (
                <HardDriveDownloadIcon className="mr-1" />
              )}
              导入目录
            </Button>
          </form>
        </CardContent>
      </Card>

      <Card className="rounded-3xl">
        <CardHeader>
          <CardTitle>CPA Token</CardTitle>
          <CardDescription>
            高级入口。若你不清楚 CPA 是什么，通常不用这里。
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            className="space-y-4"
            onSubmit={importCpaForm.handleSubmit(async (values) => {
              await onImportCpa(
                normalizeOptional(values.path),
                normalizeOptional(values.alias),
              );
            })}
          >
            <FieldGroup>
              <Field>
                <FieldLabel htmlFor="import-cpa-path">CPA 路径</FieldLabel>
                <FieldDescription>可留空。也可选 JSON 或目录。</FieldDescription>
                <div className="flex gap-2">
                  <Input
                    id="import-cpa-path"
                    placeholder="可选：CPA token 路径"
                    {...importCpaForm.register("path")}
                  />
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() =>
                      void pickJsonFile((value) =>
                        importCpaForm.setValue("path", value, {
                          shouldDirty: true,
                          shouldValidate: true,
                        }),
                      )
                    }
                  >
                    <FolderOpenIcon className="mr-1" />
                    浏览
                  </Button>
                </div>
                <FieldError errors={[importCpaForm.formState.errors.path]} />
              </Field>

              <Field>
                <FieldLabel htmlFor="import-cpa-alias">别名</FieldLabel>
                <Input
                  id="import-cpa-alias"
                  placeholder="可留空"
                  {...importCpaForm.register("alias")}
                />
                <FieldError errors={[importCpaForm.formState.errors.alias]} />
              </Field>
            </FieldGroup>

            <Button type="submit" disabled={busyKind === "importCpa"}>
              {busyKind === "importCpa" ? (
                <Loader2Icon className="mr-1 animate-spin" />
              ) : (
                <KeyRoundIcon className="mr-1" />
              )}
              导入 CPA
            </Button>
          </form>
        </CardContent>
      </Card>

      <Card className="rounded-3xl">
        <CardHeader>
          <CardTitle>重建 registry</CardTitle>
          <CardDescription>
            purge 后重建账号索引。不会保存 token 正文。
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            className="space-y-4"
            onSubmit={rebuildRegistryForm.handleSubmit(async (values) => {
              await onRebuildRegistry(normalizeOptional(values.path));
            })}
          >
            <Field>
              <FieldLabel htmlFor="rebuild-registry-path">源目录</FieldLabel>
              <FieldDescription>可留空。留空则由 codex-auth 自行处理。</FieldDescription>
              <div className="flex gap-2">
                <Input
                  id="rebuild-registry-path"
                  placeholder="可选：指定重建目录"
                  {...rebuildRegistryForm.register("path")}
                />
                <Button
                  type="button"
                  variant="outline"
                  onClick={() =>
                    void pickDirectory((value) =>
                      rebuildRegistryForm.setValue("path", value, {
                        shouldDirty: true,
                        shouldValidate: true,
                      }),
                    )
                  }
                >
                  <FolderOpenIcon className="mr-1" />
                  浏览
                </Button>
              </div>
              <FieldError errors={[rebuildRegistryForm.formState.errors.path]} />
            </Field>

            <Button
              type="submit"
              variant="destructive"
              disabled={busyKind === "rebuildRegistry"}
            >
              {busyKind === "rebuildRegistry" ? (
                <Loader2Icon className="mr-1 animate-spin" />
              ) : (
                <RefreshCcwIcon className="mr-1" />
              )}
              purge 并重建
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}

function SettingsPage({
  snapshot,
  busyKind,
  onToggleAutoSwitch,
  onToggleUsageApi,
  onLaunchLogin,
}: {
  snapshot: AppSnapshotDto;
  busyKind: AppAction["kind"] | null;
  onToggleAutoSwitch: (enabled: boolean) => Promise<void>;
  onToggleUsageApi: (enabled: boolean) => Promise<void>;
  onLaunchLogin: (deviceAuth: boolean) => Promise<void>;
}) {
  const activeAccount = snapshot.dashboard.activeAccount;
  const accountNameByChatgptAccountId = buildAccountNameByChatgptAccountId(
    snapshot.registry.accounts,
  );
  const activeAccountFallbackName = activeAccount?.chatgptAccountId
    ? accountNameByChatgptAccountId.get(activeAccount.chatgptAccountId)
    : undefined;

  return (
    <div className="grid gap-4 xl:grid-cols-2">
      <Card className="rounded-3xl">
        <CardHeader>
          <CardTitle>自动切换</CardTitle>
          <CardDescription>
            GUI 打开时检测配额与账号状态，耗尽后切到下一个可用账号。
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between gap-4 rounded-2xl border border-border/70 px-4 py-3">
            <div>
              <div className="font-medium">auto-switch</div>
              <div className="text-sm text-muted-foreground">
                当前：{snapshot.registry.autoSwitchEnabled ? "已开启" : "已关闭"}
              </div>
            </div>
            <Switch
              checked={snapshot.registry.autoSwitchEnabled}
              onCheckedChange={(checked) =>
                void onToggleAutoSwitch(Boolean(checked))
              }
              disabled={busyKind === "autoSwitch"}
            />
          </div>

          <Alert>
            <InfoIcon />
            <AlertTitle>说明</AlertTitle>
            <AlertDescription>
              不使用 Windows 计划任务。GUI 关闭后不会自动切换。切换成功后仍建议重启 Codex CLI / Codex App。
            </AlertDescription>
          </Alert>
        </CardContent>
      </Card>

      <Card className="rounded-3xl">
        <CardHeader>
          <CardTitle>Usage API 模式</CardTitle>
          <CardDescription>
            API 模式下，额度与团队信息更准。开启前会二次确认风险。
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between gap-4 rounded-2xl border border-border/70 px-4 py-3">
            <div>
              <div className="font-medium">usage API mode</div>
              <div className="text-sm text-muted-foreground">
                当前：{formatModeLabel(snapshot.registry.usageMode)}
              </div>
            </div>
            <Switch
              checked={snapshot.registry.usageMode === "api"}
              onCheckedChange={(checked) =>
                void onToggleUsageApi(Boolean(checked))
              }
              disabled={busyKind === "usageApi"}
            />
          </div>

          <Alert>
            <ShieldAlertIcon />
            <AlertTitle>风险提醒</AlertTitle>
            <AlertDescription>
              GUI 不保存 token 正文。但 API 模式会调用 codex-auth 获取更实时的账号信息。
            </AlertDescription>
          </Alert>
        </CardContent>
      </Card>

      <Card className="rounded-3xl xl:col-span-2">
        <CardHeader>
          <CardTitle>登录方式</CardTitle>
          <CardDescription>
            网页登录会打开外部窗口，并按 Codex 提示拉起系统浏览器。
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap gap-3">
            <Button
              disabled={busyKind === "launchLogin"}
              onClick={() => void onLaunchLogin(false)}
            >
              {busyKind === "launchLogin" ? (
                <Loader2Icon className="mr-1 animate-spin" />
              ) : (
                <KeyRoundIcon className="mr-1" />
              )}
              网页登录
            </Button>
            <Button
              variant="outline"
              disabled={busyKind === "launchLogin"}
              onClick={() => void onLaunchLogin(true)}
            >
              {busyKind === "launchLogin" ? (
                <Loader2Icon className="mr-1 animate-spin" />
              ) : (
                <LaptopMinimalCheckIcon className="mr-1" />
              )}
              设备码登录
            </Button>
          </div>

          <div className="grid gap-3 md:grid-cols-3">
            <MetricCard
              label="当前激活账号"
              value={activeAccount?.email ?? "暂无"}
              detail={activeAccount ? getAccountStatusLabel(activeAccount) : "未激活"}
            />
            <MetricCard
              label="账号类型"
              value={activeAccount ? getAccountTypeLabel(activeAccount) : "未知"}
              detail={
                activeAccount
                  ? getAccountSpaceLabel(activeAccount, activeAccountFallbackName)
                  : "未获取"
              }
            />
            <MetricCard
              label="切换后动作"
              value="重启 Codex"
              detail="CLI / App 都建议重启"
            />
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

function DiagnosticsPage({
  snapshot,
  statusLog,
  statusBusy,
  onRunStatus,
  onOpenPath,
}: {
  snapshot: AppSnapshotDto;
  statusLog: CommandExecutionDto | null;
  statusBusy: boolean;
  onRunStatus: () => void;
  onOpenPath: (target: string) => Promise<void>;
}) {
  const directoryItems = [
    {
      key: "codexRoot",
      label: "~/.codex",
      value: snapshot.diagnostics.directories.codexRoot,
    },
    {
      key: "accountsDir",
      label: "账号目录",
      value: snapshot.diagnostics.directories.accountsDir,
    },
    {
      key: "sessionsDir",
      label: "会话目录",
      value: snapshot.diagnostics.directories.sessionsDir,
    },
    {
      key: "registryFile",
      label: "registry.json",
      value: snapshot.diagnostics.directories.registryPath,
    },
    {
      key: "logsDir",
      label: "GUI 日志",
      value: snapshot.diagnostics.directories.appLogDir,
    },
    {
      key: "logsFile",
      label: "日志文件",
      value: snapshot.diagnostics.directories.appLogFile,
    },
  ] as const;
  const latestRefreshTotal = snapshot.diagnostics.performance.find(
    (item) => item.label === "refresh.total",
  );
  const latestRefreshCli = snapshot.diagnostics.performance.find(
    (item) => item.label === "refresh.cli",
  );
  const latestRegistryRead = snapshot.diagnostics.performance.find(
    (item) => item.label === "refresh.registry-read",
  );

  return (
    <div className="space-y-6">
      <Card className="rounded-3xl">
        <CardHeader className="gap-4 md:flex-row md:items-start md:justify-between">
          <div className="space-y-1">
            <CardTitle>codex-auth status</CardTitle>
            <CardDescription>
              原始执行 `codex-auth status`。附带超时保护。
            </CardDescription>
          </div>
          <Button
            variant="outline"
            onClick={onRunStatus}
            disabled={statusBusy}
          >
            {statusBusy ? (
              <Loader2Icon className="mr-1 animate-spin" />
            ) : (
              <RefreshCcwIcon className="mr-1" />
            )}
            执行 status
          </Button>
        </CardHeader>
        <CardContent>
          {statusLog ? (
            <CommandEntryCard log={statusLog} />
          ) : (
            <Empty className="rounded-2xl border border-dashed">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <LogsIcon />
                </EmptyMedia>
                <EmptyTitle>还没有 status 日志</EmptyTitle>
                <EmptyDescription>点上方按钮执行一次状态查询。</EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </CardContent>
      </Card>

      <Card className="rounded-3xl">
        <CardHeader>
          <CardTitle>环境与路径</CardTitle>
          <CardDescription>
            Node 版本、codex-auth 路径、codex 路径、目录跳转。
          </CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4 xl:grid-cols-[minmax(0,1.5fr)_minmax(320px,1fr)]">
          <div className="grid gap-4 md:grid-cols-3">
            {snapshot.diagnostics.envChecks.map((check) => (
              <EnvCheckCard key={check.key} check={check} />
            ))}
          </div>

          <div className="space-y-3">
            {directoryItems.map((item) => (
              <button
                key={item.key}
                type="button"
                className="flex w-full items-center justify-between gap-4 rounded-2xl border border-border/70 px-4 py-3 text-left transition hover:bg-muted/50"
                onClick={() => void onOpenPath(item.key)}
              >
                <div className="font-medium">{item.label}</div>
                <div className="min-w-0 flex-1 truncate text-right text-sm text-muted-foreground">
                  {item.value}
                </div>
              </button>
            ))}
          </div>
        </CardContent>
      </Card>

      <Card className="rounded-3xl">
        <CardHeader>
          <CardTitle>刷新性能</CardTitle>
          <CardDescription>
            用来定位刷新卡顿。点右上角刷新后看这里的耗时。
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-3 md:grid-cols-3">
            <MetricCard
              label="刷新总耗时"
              value={
                latestRefreshTotal ? `${latestRefreshTotal.durationMs} ms` : "暂无"
              }
              detail={latestRefreshTotal?.detail ?? "还没有刷新性能记录"}
            />
            <MetricCard
              label="CLI 耗时"
              value={latestRefreshCli ? `${latestRefreshCli.durationMs} ms` : "暂无"}
              detail={latestRefreshCli?.detail ?? "`codex-auth list --debug`"}
            />
            <MetricCard
              label="registry 读取"
              value={
                latestRegistryRead
                  ? `${latestRegistryRead.durationMs} ms`
                  : "暂无"
              }
              detail={latestRegistryRead?.detail ?? "解析 registry.json"}
            />
          </div>

          {snapshot.diagnostics.performance.length ? (
            <div className="overflow-hidden rounded-2xl border border-border/70">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>阶段</TableHead>
                    <TableHead>耗时</TableHead>
                    <TableHead>时间</TableHead>
                    <TableHead>说明</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {snapshot.diagnostics.performance.slice(0, 12).map((item) => (
                    <TableRow key={`${item.label}-${item.timestampMs}`}>
                      <TableCell className="font-medium">{item.label}</TableCell>
                      <TableCell>{item.durationMs} ms</TableCell>
                      <TableCell>{formatTimestamp(item.timestampMs)}</TableCell>
                      <TableCell className="break-all text-muted-foreground">
                        {item.detail}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          ) : null}
        </CardContent>
      </Card>

      <Card className="rounded-3xl">
        <CardHeader>
          <CardTitle>最近命令日志</CardTitle>
          <CardDescription>
            最近命令执行记录。看 stdout / stderr / exit code。
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {snapshot.diagnostics.recentLogs.length ? (
            snapshot.diagnostics.recentLogs.map((log) => (
              <CommandEntryCard
                key={log.id}
                log={log}
                title={commandCategoryLabel(log.category)}
              />
            ))
          ) : (
            <Empty className="rounded-2xl border border-dashed">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <LogsIcon />
                </EmptyMedia>
                <EmptyTitle>暂无日志</EmptyTitle>
                <EmptyDescription>
                  执行一次刷新、切换或 status 后，这里会累积日志。
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function UsageCard({
  title,
  usage,
}: {
  title: string;
  usage: AccountItem["primaryUsage"] | null;
}) {
  const value = usage?.remainingPercent ?? null;

  return (
    <Card className="rounded-3xl">
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardDescription>
          重置：{formatShortTimestamp(usage?.resetsAtMs)}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="text-5xl font-semibold tracking-tight">
          {formatPercent(value)}
        </div>
        <Progress value={value ?? 0}>
          <div className="flex w-full items-center gap-2">
            <ProgressLabel>{title}</ProgressLabel>
            <div className="ml-auto text-sm text-muted-foreground">
              {formatPercent(value)}
            </div>
          </div>
        </Progress>
      </CardContent>
    </Card>
  );
}

function EnvCheckCard({
  check,
}: {
  check: AppSnapshotDto["dashboard"]["envChecks"][number];
}) {
  return (
    <Card className="rounded-3xl">
      <CardHeader>
        <div className="flex items-center justify-between gap-3">
          <CardTitle>{check.label}</CardTitle>
          <Badge variant={check.ok ? "secondary" : "destructive"}>
            {check.ok ? "正常" : "异常"}
          </Badge>
        </div>
        <CardDescription>{check.message}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-2 text-sm">
        <div className="font-medium">{fallbackText(check.version, "未获取版本")}</div>
        <div className="break-all text-muted-foreground">
          {fallbackText(check.path, "未找到路径")}
        </div>
      </CardContent>
    </Card>
  );
}

function MetricCard({
  label,
  value,
  detail,
}: {
  label: string;
  value: string;
  detail?: string;
}) {
  return (
    <div className="rounded-3xl border border-border/70 px-4 py-4">
      <div className="text-sm text-muted-foreground">{label}</div>
      <div className="mt-2 break-all text-2xl font-semibold tracking-tight">
        {value}
      </div>
      {detail ? (
        <div className="mt-2 text-sm text-muted-foreground">{detail}</div>
      ) : null}
    </div>
  );
}

function SidebarStatCard({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-3xl border border-sidebar-border/70 bg-sidebar px-4 py-3">
      <div className="text-xs tracking-[0.18em] text-muted-foreground uppercase">
        {label}
      </div>
      <div className="mt-2 text-2xl font-semibold tracking-tight">{value}</div>
    </div>
  );
}

function CommandEntryCard({
  log,
  title,
}: {
  log: CommandExecutionDto;
  title?: string;
}) {
  const state = logState(log);
  const defaultTab = log.stderr ? "stderr" : "stdout";

  return (
    <div className="rounded-2xl border border-border/70 p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <div className="text-sm font-medium text-muted-foreground">
            {title ?? commandCategoryLabel(log.category)}
          </div>
          <div className="break-all text-xl font-semibold">{log.displayCommand}</div>
          <div className="text-sm text-muted-foreground">
            {formatTimestamp(log.startedAtMs)} · {log.durationMs} 毫秒
          </div>
        </div>

        <div className="flex flex-wrap gap-2">
          <Badge variant={state.variant}>{state.label}</Badge>
          <Badge variant="outline">退出码 {log.exitCode ?? "?"}</Badge>
        </div>
      </div>

      <div className="mt-4 grid gap-3 lg:grid-cols-3">
        <MetricCard
          label="类别"
          value={commandCategoryLabel(log.category)}
          detail={log.timedOut ? "命令已超时" : "已完成"}
        />
        <MetricCard
          label="可执行文件"
          value={log.executablePath}
          detail=""
        />
        <MetricCard
          label="工作目录"
          value={log.cwd}
          detail=""
        />
      </div>

      <Tabs defaultValue={defaultTab} className="mt-4">
        <TabsList variant="line">
          <TabsTrigger value="stdout">stdout</TabsTrigger>
          <TabsTrigger value="stderr">stderr</TabsTrigger>
        </TabsList>
        <TabsContent value="stdout">
          <pre className="max-h-72 overflow-auto rounded-2xl bg-muted/50 p-4 text-xs leading-6 whitespace-pre-wrap break-all">
            {log.stdout || "无 stdout"}
          </pre>
        </TabsContent>
        <TabsContent value="stderr">
          <pre className="max-h-72 overflow-auto rounded-2xl bg-muted/50 p-4 text-xs leading-6 whitespace-pre-wrap break-all">
            {log.stderr || "无 stderr"}
          </pre>
        </TabsContent>
      </Tabs>
    </div>
  );
}

function BootSkeleton() {
  return (
    <div className="min-h-svh bg-background p-4 md:p-6">
      <div className="grid gap-4 md:grid-cols-[260px_minmax(0,1fr)]">
        <div className="space-y-4">
          <Skeleton className="h-48 rounded-3xl" />
          <Skeleton className="h-64 rounded-3xl" />
        </div>
        <div className="space-y-4">
          <Skeleton className="h-28 rounded-3xl" />
          <div className="grid gap-4 xl:grid-cols-2">
            <Skeleton className="h-64 rounded-3xl" />
            <Skeleton className="h-64 rounded-3xl" />
          </div>
          <Skeleton className="h-96 rounded-3xl" />
        </div>
      </div>
    </div>
  );
}
