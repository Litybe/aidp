function __poolBud() {
  return (typeof budget === "object" && budget && typeof budget.remaining === "function") ? budget.remaining() : Infinity;
}
function __poolSem(n) {
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
function pool(n, floor) {
  const s = __poolSem(n);
  return { async run(fn) {
    if (__poolBud() < floor) return { status: "cancelled", summary: "non dispatche : budget restant (" + __poolBud() + ") sous le plancher (" + floor + ")" };
    await s.acquire(); try { return await fn(); } finally { s.release(); }
  } };
}
