/// One line of the text with its heading and item status resolved outside fences.
pub struct ClassifiedLine<'a> {
    pub text: &'a str,
    pub heading: Option<String>,
    pub item: bool,
}
