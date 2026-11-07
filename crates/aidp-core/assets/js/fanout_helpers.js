function shards(v, max, fold) {
  let a = Array.isArray(v) ? v.filter(x => typeof x === "string" && x.trim()).map(x => x.trim()) : [];
  if (!a.length) return ["(whole stage scope — fan-out source empty, running a single agent)"];
  if (a.length > max) {
    if (fold === false) {
      // C8 (Lot 3, Capitalisation): `fan_out.fold: false` — `max` is a hard
      // ceiling on item count (never folded into fewer, larger shards), so a
      // dynamic `from:` resolving to more items than `max` drops the excess
      // rather than exploding the sub-agent count. Concurrency among the
      // (<=max) shards that remain is `pool(max, ...)`'s job, not this cap's.
      log("fan-out: dropping " + (a.length - max) + " item(s) beyond the hard cap (max=" + max + ", fold=false)");
      a = a.slice(0, max);
    } else {
      log("fan-out: folding " + (a.length - max + 1) + " shard(s) into the last one (max=" + max + ")");
      const head = a.slice(0, max - 1);
      head.push(a.slice(max - 1).join(" + "));
      a = head;
    }
  }
  return a;
}
function sigArr(stage, prop) {
  const r = results[stage];
  const v = r && r.signals ? r.signals[prop] : null;
  return Array.isArray(v) ? v : [];
}
function shardNote(s, i, n) {
  return "\n\n## Fan-out shard " + (i + 1) + "/" + n + ": " + s +
    "\n\nThis stage is fanned out across " + n + " parallel sub-agents sharing the SAME working tree. Handle ONLY this shard's slice of the work. Create or modify only files belonging to this shard; if a file must be shared with another shard (e.g. DI registration, a common csproj), do NOT touch it — list it under `follow_ups` so the merge step handles it. In `outputs`, name the files you actually changed.";
}
function mergeNote(hint) {
  return "\n\n## Fan-out merge\n\nYou are the MERGE step of this fanned-out stage: the shard sub-agents below already did the work in this working tree — do NOT redo it. Your job: apply any shared-file work the shards deferred (see their follow_ups), verify the combined result is coherent (build / run tests if applicable, fix integration breakage), and produce this stage's canonical outputs and reply envelope covering ALL shards." +
    (hint ? "\n\n" + hint : "");
}
function fanin(rs, sh) {
  let t = "\n\n## Shard results\n\n";
  for (let i = 0; i < rs.length; i++) {
    const r = rs[i];
    t += "- [" + sh[i] + "] " + (r ? __guard(JSON.stringify({ status: r.status, summary: r.summary, outputs: r.outputs || {}, signals: r.signals || {}, follow_ups: r.follow_ups || [] }), "fanin:" + sh[i]) : "null (dispatch failed — investigate and redo this shard's work yourself)") + "\n";
  }
  return t;
}
