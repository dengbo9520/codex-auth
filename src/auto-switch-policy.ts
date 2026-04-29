interface RefreshAutomationCommand {
  success: boolean;
  stdout: string;
}

interface RefreshAutomationRegistry {
  accounts: readonly unknown[];
}

export interface RefreshAutomationResult {
  command: RefreshAutomationCommand;
  registry: RefreshAutomationRegistry;
}

const USAGE_REFRESH_DONE_MARKER = "[debug] usage refresh done:";

export function refreshResultSupportsAutomation(result: RefreshAutomationResult) {
  if (result.command.success) {
    return true;
  }

  return (
    result.registry.accounts.length > 0 &&
    result.command.stdout.includes(USAGE_REFRESH_DONE_MARKER)
  );
}
