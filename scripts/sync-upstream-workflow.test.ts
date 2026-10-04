import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const workflow: any = Bun.YAML.parse(
  readFileSync(new URL("../.github/workflows/sync-upstream.yml", import.meta.url), "utf8"),
);
const steps = workflow.jobs.sync.steps;
const stepIf = (name: string): string => steps.find((step: any) => step.name === name)?.if;

describe("sync catch-all issue", () => {
  // The conflict and guardrail paths fail the job on purpose and open their own
  // issue. Without an explicit exclusion the catch-all also fires for them, so
  // every conflicted sync files a second "failed unexpectedly" issue (#64) for a
  // failure that was already reported.
  test("stays silent when the conflict path already opened an issue", () => {
    expect(stepIf("Open failure issue (catch-all)")).toContain(
      "steps.merge.outputs.merge_state != 'conflicts'",
    );
  });

  test("stays silent when the marker guardrail already opened an issue", () => {
    expect(stepIf("Open failure issue (catch-all)")).toContain("steps.verify.outcome != 'failure'");
  });

  test("still reports failures no other path claims", () => {
    expect(stepIf("Open failure issue (catch-all)")).toContain("failure()");
  });

  test("the conflict path opens its own issue", () => {
    expect(stepIf("Open sync-conflict issue")).toBe("steps.merge.outputs.merge_state == 'conflicts'");
    expect(stepIf("Fail job on conflict")).toBe("steps.merge.outputs.merge_state == 'conflicts'");
  });
});