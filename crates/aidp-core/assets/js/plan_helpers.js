// Counting semaphore: the stage's `plan_tasks.max`. The runtime queues excess
// dispatches on its own, but that cap is global — this one is the template's.
function sem(n) {
  let free = Math.max(1, n);
  const q = [];
  return {
    async acquire() {
      if (free > 0) { free--; return; }
      await new Promise(r => q.push(r));
    },
    release() {
      const next = q.shift();
      if (next) next(); else free++;
    }
  };
}
// One plan task. Started eagerly (so it is ready the moment its deps are) and
// resolved through the task store, which is why the emitter writes tasks in
// topological order. Never throws and never resolves to null: a dead upstream
// task or a failed dispatch becomes a `failed` envelope, so the stage's pause
// check reports it instead of the whole workflow crashing on an unhandled
// rejection.
// C8 (Lot 3, Capitalisation): `gate` is now always a `pool(...)` (see
// `emit_plan_stage`'s `__sm_<id>`), never a bare `sem(...)` — so it is used
// through `gate.run(fn)` (admission-budget check + acquire/release, all in
// one) rather than `gate.acquire()`/`gate.release()` directly. `sem()` itself
// is untouched; only how its result is consumed here would need to change if
// something ever passed one to `runTask` again.
// C9 (Lot 3, Capitalisation): `mutex` is a NEW third parameter, disjoint from
// `deps`. `deps` is `aidp-plan.json`'s real `needs` graph: a `blocked`/
// `failed` entry there must cancel this task (the loop below, unchanged).
// `mutex` is an ordering `aidp` inserted on its own because this task and an
// upstream one write the same file — the lock is still respected (`await
// store[m]` below strictly serializes on it, so the shared working tree is
// never raced), but a failed mutex predecessor must NOT cancel this task:
// it only leaves the tree possibly half-edited, which is a fact this task
// needs to know, not a reason to skip it. See plan.rs's `ScheduledTask` doc
// comment for the full rationale.
async function runTask(id, deps, mutex, store, gate, promptText, opts, tries) {
  const ups = [];
  for (const d of deps) {
    const prev = await store[d];
    const env = prev ? prev.result : null;
    if (!env || env.status === "blocked" || env.status === "failed" || env.status === "cancelled") {
      return { id: id, skipped: true, result: { status: "failed", summary: "task " + id + " not run: upstream task " + d + " did not complete" } };
    }
    ups.push([d, env]);
  }
  const mutexWarnings = [];
  for (const m of mutex) {
    const prev = await store[m];
    const env = prev ? prev.result : null;
    if (!env || env.status === "blocked" || env.status === "failed" || env.status === "cancelled") {
      mutexWarnings.push("- " + m + " (serialisee avant toi sur un fichier partage) n'a PAS abouti : verifie l'etat de ces fichiers avant d'editer, elle a pu les laisser a moitie modifies.");
    }
  }
  try {
    let p = promptText + taskUp(ups);
    if (mutexWarnings.length) p += "\n\n## Verrou fichier — predecesseur en echec\n\n" + mutexWarnings.join("\n") + "\n";
    const r = await gate.run(() => (tries > 0 && typeof dispatch === "function") ? dispatch(p, opts, tries) : safeAgent(p, opts));
    return { id: id, result: r || { status: "failed", summary: "task " + id + ": dispatch returned nothing" } };
  } catch (e) {
    return { id: id, result: { status: "failed", summary: "task " + id + " threw: " + ((e && e.message) ? e.message : String(e)) } };
  }
}
// Await every task of a stage. The promises are already running; this only
// collects them in a stable order.
async function settle(store, ids) {
  const out = [];
  for (const id of ids) out.push(await store[id]);
  return out;
}
function taskUp(ups) {
  if (!ups.length) return "";
  let t = "\n\n## Upstream tasks — already applied in this working tree\n\n";
  for (const u of ups) {
    const e = u[1];
    t += "- [" + u[0] + "] " + __guard(JSON.stringify({ status: e.status, summary: e.summary, outputs: e.outputs || {}, follow_ups: e.follow_ups || [] }), "taskUp:" + u[0]) + "\n";
  }
  return t + "\nTheir changes are on disk: build on them, do not redo them.\n";
}
function logTasks(stage, rs) {
  let ok = 0, ko = [];
  for (const r of rs) {
    if (r && r.result && r.result.status === "completed") ok++;
    else ko.push(r ? r.id : "?");
  }
  log(stage + ": " + ok + "/" + rs.length + " task(s) completed" + (ko.length ? " — not completed: " + ko.join(", ") : ""));
}
function taskin(rs) {
  let t = "\n\n## Task results\n\n";
  for (const r of rs) {
    const e = r ? r.result : null;
    const id = r ? r.id : "?";
    t += "- [" + id + "] " + (e ? __guard(JSON.stringify({ status: e.status, summary: e.summary, outputs: e.outputs || {}, follow_ups: e.follow_ups || [], scope_violations: e.scope_violations || [] }), "taskin:" + id) : "null (dispatch failed — investigate and redo this task's work yourself)") + "\n";
  }
  return t;
}
