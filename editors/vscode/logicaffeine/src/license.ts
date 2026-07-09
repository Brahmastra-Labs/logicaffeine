import { ExtensionContext, window } from "vscode";

const LICENSE_KEY = "logicaffeine.license";

/** Retrieve the stored verification license, if any. */
export function getLicense(context: ExtensionContext): Thenable<string | undefined> {
  return context.secrets.get(LICENSE_KEY);
}

/**
 * Prompt for and store the verification license key in SecretStorage —
 * never in settings.json, where a license would sync in plain text.
 */
export async function promptAndStoreLicense(context: ExtensionContext): Promise<boolean> {
  const value = await window.showInputBox({
    title: "LOGOS verification license",
    prompt: "License keys are Stripe subscription ids (sub_…) — see logicaffeine.com/pricing",
    password: true,
    ignoreFocusOut: true,
    validateInput: (input) =>
      input.trim().length === 0 ? "Enter a license key, or press Escape to cancel" : undefined,
  });
  if (value === undefined) {
    return false;
  }
  const trimmed = value.trim();
  if (trimmed.length === 0) {
    await context.secrets.delete(LICENSE_KEY);
    window.showInformationMessage("LOGOS license cleared.");
    return false;
  }
  await context.secrets.store(LICENSE_KEY, trimmed);
  window.showInformationMessage("LOGOS license stored securely.");
  return true;
}
