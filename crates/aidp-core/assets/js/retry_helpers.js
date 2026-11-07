function bud() {
  return (typeof budget === "object" && budget && typeof budget.remaining === "function") ? budget.remaining() : Infinity;
}
async function dispatch(promptText, opts, tries) {
  let r;
  for (let k = 0; k <= tries; k++) {
    const p = k === 0 ? promptText : promptText + "\n\n## Retry " + k + ": the previous attempt returned status=failed — diagnose and fix the cause before retrying.";
    r = await safeAgent(p, opts);
    if (!r || r.status !== "failed") return r;
    if (r.signals && r.signals.skills_not_loaded) return r;
    if (bud() < 50000) return r;
  }
  return r;
}
