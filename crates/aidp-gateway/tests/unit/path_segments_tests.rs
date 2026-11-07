use super::{decode, split};

const ENCODED_STEP: &str = "ci%40dev%2Fci_watch";
const STEP: &str = "ci@dev/ci_watch";
const ENCODED_TASK: &str = "implement%23t1";
const TASK: &str = "implement#t1";
const PLAIN: &str = "LOC-99";
const MALFORMED: &str = "50%zz%";
const ACCENTED: &str = "d%C3%A9ploiement";
const ACCENTED_DECODED: &str = "déploiement";

#[test]
fn decodes_the_reserved_characters_of_a_step_or_task_id() {
    assert_eq!(decode(ENCODED_STEP), STEP);
    assert_eq!(decode(ENCODED_TASK), TASK);
    assert_eq!(decode(ACCENTED), ACCENTED_DECODED);
}

#[test]
fn leaves_plain_and_malformed_segments_as_written() {
    assert_eq!(decode(PLAIN), PLAIN);
    assert_eq!(decode(MALFORMED), MALFORMED);
}

#[test]
fn splits_before_decoding_so_an_encoded_slash_stays_in_its_segment() {
    let path = format!("flows/{PLAIN}/steps/{ENCODED_STEP}/retry");
    assert_eq!(split(&path)[3], STEP);
    assert_eq!(split(&path).len(), 5);
}
