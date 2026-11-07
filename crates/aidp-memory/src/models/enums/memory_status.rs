string_enum!(
    /// Where a memory stands in its life cycle.
    MemoryStatus, "status", {
        Proposed => "proposed",
        Active => "active",
        Stale => "stale",
        Archived => "archived",
        Superseded => "superseded",
    }
);
