/// Words ignored in queries and similarity: French and English function words.
pub const STOPWORDS: &[&str] = &[
    "le", "la", "les", "un", "une", "des", "du", "de", "et", "ou", "en", "au", "aux", "ce", "ces",
    "cet", "cette", "que", "qui", "quoi", "dans", "sur", "sous", "par", "pour", "avec", "sans",
    "est", "sont", "etre", "avoir", "il", "elle", "ils", "elles", "on", "se", "sa", "son", "ses",
    "ne", "pas", "plus", "comment", "quel", "quelle", "quels", "quelles", "ou", "the", "a", "an",
    "of", "to", "in", "on", "for", "and", "or", "is", "are", "be", "how", "what", "which", "with",
    "by", "at", "as", "it", "this", "that", "from", "do", "does",
];
