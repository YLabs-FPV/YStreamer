export const WEBSITE = "https://ystreamer.yarosfpv.com";
export const SOURCE = "https://github.com/YLabs-FPV/YStreamer";
export const NEW_ISSUE = `${SOURCE}/issues/new/choose`;

export interface BugReportFields {
  version?: string;
  hardware?: string;
  power?: string;
  device?: string;
}

export function bugReportUrl(fields: BugReportFields): string {
  const query = new URLSearchParams({ template: "bug_report.yml" });
  for (const [id, value] of Object.entries(fields)) {
    if (value) query.set(id, value);
  }
  return `${SOURCE}/issues/new?${query}`;
}
