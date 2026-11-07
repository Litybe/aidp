//! Ticket key carried by a branch name (`feat/LOC-12-titre` → `LOC-12`).

/// First `PREFIX-NUMBER` pair of the branch name that reads as a ticket key, if any.
pub fn ticket_key_from_branch(branch: &str) -> Option<String> {
    for segment in branch.split(|c: char| !c.is_ascii_alphanumeric() && c != '-') {
        let parts: Vec<&str> = segment.split('-').collect();
        for pair in parts.windows(2) {
            let candidate = format!("{}-{}", pair[0], pair[1]);
            if aidp_core::knowledge::is_ticket_key(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}
