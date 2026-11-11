string_enum!(
    /// What ingestion did with one knowledge proposal.
    ProposalOutcome, "proposal outcome", {
        Added => "added",
        Merged => "merged",
        Inboxed => "inboxed",
        Updated => "updated",
        Superseded => "superseded",
    }
);
