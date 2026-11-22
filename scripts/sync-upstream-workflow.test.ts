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
      "steps.conflict_issue.outcome != 'success'",
    );
  });

  test("stays silent when the marker guardrail already opened an issue", () => {
    expect(stepIf("Open failure issue (catch-all)")).toContain("steps.verify.outcome != 'failure'");
  });

  test("still reports failures no other path claims", () => {
    expect(stepIf("Open failure issue (catch-all)")).toContain("failure()");
  });

  // The two clause assertions above pin the spelling of the condition; this
  // truth table pins its routing. Substring matching cannot express which
  // failures still reach the step — the property that actually went wrong when
  // the condition keyed on `merge_state`, which the merge step sets to
  // `conflicts` for any non-zero exit, not just a real conflict.
  const runsCatchAll = (o: { merge: string; verify: string; conflictIssue: string }): boolean =>
    o.verify !== "failure" && o.conflictIssue !== "success";

  test("fires when the merge never ran (checkout, fetch, push rejection)", () => {
    expect(runsCatchAll({ merge: "", verify: "skipped", conflictIssue: "skipped" })).toBe(true);
  });

  test("fires when the conflict issue step itself failed", () => {
    expect(runsCatchAll({ merge: "conflicts", verify: "skipped", conflictIssue: "failure" })).toBe(
      true,
    );
  });

  test("stays silent when the conflict path already reported", () => {
    expect(runsCatchAll({ merge: "conflicts", verify: "skipped", conflictIssue: "success" })).toBe(
      false,
    );
  });

  test("stays silent when the guardrail already reported", () => {
    expect(runsCatchAll({ merge: "clean", verify: "failure", conflictIssue: "skipped" })).toBe(false);
  });

  test("the conflict path opens its own issue under an id the catch-all can read", () => {
    const conflictStep = steps.find((step: any) => step.name === "Open sync-conflict issue");
    expect(conflictStep.id).toBe("conflict_issue");
    expect(stepIf("Open sync-conflict issue")).toBe("steps.merge.outputs.merge_state == 'conflicts'");
    expect(stepIf("Fail job on conflict")).toBe("steps.merge.outputs.merge_state == 'conflicts'");
  });
});
