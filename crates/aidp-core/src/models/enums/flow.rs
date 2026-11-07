/// Where compilation goes after a stage has been routed.
pub(crate) enum Flow {
    Next(usize),
    Stop,
}
