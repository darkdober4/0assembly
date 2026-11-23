//! Catalogue des langages reconnus par `0ass`.
//!
//! Chaque langage est décrit par des données (pas de code) : extensions, syntaxe
//! de commentaires et de chaînes, motifs de définition de fonction, mots-clés de
//! boucle, appels d'allocation et appels « chauds ». Le scanneur générique
//! (`crate::scan`) s'appuie uniquement sur ces descripteurs, ce qui permet
//! d'ajouter un langage sans écrire de logique.

/// Style de délimitation du corps d'une fonction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyStyle {
    /// Corps délimité par des accolades (`{ ... }`).
    Braces,
    /// Corps délimité par l'indentation (Python).
    Indent,
    /// Corps terminé par un mot-clé (`end`).
    End,
    /// Corps d'une expression : jusqu'à la définition suivante ou la fin du fichier.
    Expression,
}

/// Famille de langage : partage les valeurs par défaut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    CLike,
    Python,
    Ruby,
    Lua,
    Shell,
    Haskell,
    Ocaml,
    Erlang,
    Elixir,
    Julia,
    R,
    Fortran,
    Matlab,
    Perl,
    PowerShell,
    Sql,
}

/// Descripteur statique d'un langage.
#[derive(Debug, Clone, Copy)]
pub struct LangProfile {
    /// Identifiant court (`rust`, `c`, `python`…).
    pub id: &'static str,
    /// Nom lisible.
    pub name: &'static str,
    /// Extensions de fichier, sans le point.
    pub extensions: &'static [&'static str],
    /// Marqueurs de commentaire sur une ligne.
    pub line_comments: &'static [&'static str],
    /// Couple (ouverture, fermeture) d'un commentaire multiligne.
    pub block_comment: Option<(&'static str, &'static str)>,
    /// Chaînes : (délimiteur, caractère d'échappement).
    pub strings: &'static [(&'static str, &'static str)],
    /// Chaînes brutes : délimiteurs sans échappement (Python `r"..."`, Rust `r#"..."#`).
    pub raw_strings: &'static [&'static str],
    /// Expressions régulières de définition de fonction : le groupe 1 est le nom.
    pub fn_patterns: &'static [&'static str],
    /// Style de délimitation du corps.
    pub body: BodyStyle,
    /// Mots-clés de boucle.
    pub loops: &'static [&'static str],
    /// Appels d'allocation (heuristique « allocation dans une boucle »).
    pub alloc_calls: &'static [&'static str],
    /// Appels fréquents dans une boucle (heuristique « boucle chaude »).
    pub hot_calls: &'static [&'static str],
    /// Mots-clés de branchement.
    pub branches: &'static [&'static str],
    /// Le langage a-t-il des littéraux de caractère (`'x'`) ?
    /// (`false` pour Rust, où `'` introduit aussi les durées de vie.)
    pub char_literals: bool,
}

/// Valeurs par défaut d'une famille « à accolades ».
const CLINE_COMMENTS: &[&str] = &["//"];
const CLINE_BLOCK: (&str, &str) = ("/*", "*/");
const CLINE_STRINGS: &[(&str, &str)] = &[("\"", "\\"), ("'", "\\")];
const CLINE_BRANCHES: &[&str] = &[
    "if", "else", "switch", "case", "match", "when", "unless", "elif", "ternary",
];
const CLINE_LOOPS: &[&str] = &["for", "while", "do", "loop", "foreach"];
const CLINE_ALLOC: &[&str] = &[
    "new ",
    "malloc",
    "calloc",
    "realloc",
    "Vec::new",
    "vec![",
    "String::new",
    "format!",
    "emplace_back",
    "resize(",
    "reserve(",
    "realloc(",
];

/// Motif générique d'une définition à accolades : `type nom(params) {`.
const GENERIC_FN: &str = r"(?m)^[ \t]*(?:[A-Za-z_][\w:<>,\.\*\&\[\]\s]*?[\s\*&])?([A-Za-z_]\w*)\s*\([^;{}]*\)\s*(?:const\s*)?(?:noexcept\s*)?(?:->[^{;]*)?\{";

/// Noms qui ressemblent à une définition mais sont des mots-clés : à ignorer.
pub const RESERVED_NAMES: &[&str] = &[
    "if",
    "else",
    "for",
    "foreach",
    "while",
    "do",
    "switch",
    "case",
    "catch",
    "try",
    "finally",
    "match",
    "when",
    "unless",
    "elif",
    "using",
    "lock",
    "synchronized",
    "fixed",
    "unsafe",
    "return",
    "sizeof",
    "typeof",
    "new",
    "delete",
    "throw",
    "except",
    "with",
    "assert",
    "yield",
    "await",
    "async",
    "lambda",
    "require",
    "include",
    "import",
    "from",
    "package",
    "namespace",
    "class",
    "struct",
    "enum",
    "interface",
    "trait",
    "impl",
    "fn",
    "func",
    "function",
    "def",
    "let",
    "var",
    "const",
    "public",
    "private",
    "protected",
    "static",
    "inline",
    "virtual",
    "override",
    "final",
    "template",
    "typedef",
    "extern",
    "export",
    "default",
    "operator",
    "this",
    "super",
    "self",
    "begin",
    "end",
    "loop",
    "sub",
    "procedure",
    "module",
    "program",
    "record",
    "object",
    "module",
];

/// Construit un profil « à accolades ».
const fn clike(
    id: &'static str,
    name: &'static str,
    extensions: &'static [&'static str],
    extra_patterns: &'static [&'static str],
) -> LangProfile {
    LangProfile {
        id,
        name,
        extensions,
        line_comments: CLINE_COMMENTS,
        block_comment: Some(CLINE_BLOCK),
        strings: CLINE_STRINGS,
        raw_strings: &[],
        fn_patterns: extra_patterns,
        body: BodyStyle::Braces,
        loops: CLINE_LOOPS,
        alloc_calls: CLINE_ALLOC,
        hot_calls: &[],
        branches: CLINE_BRANCHES,
        char_literals: true,
    }
}

/// Langages reconnus.
pub static LANGUAGES: &[LangProfile] = &[
    LangProfile {
        id: "rust",
        name: "Rust",
        extensions: &["rs"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        strings: &[("\"", "\\")],
        raw_strings: &["r\"", "r#\"", "r##\""],
        fn_patterns: &[
            r#"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+([A-Za-z_]\w*)"#,
        ],
        body: BodyStyle::Braces,
        loops: &["for ", "while ", "loop ", "iter()"],
        alloc_calls: &[
            "Vec::new",
            "vec![",
            "String::new",
            "String::from",
            "Box::new",
            "format!",
            ".to_vec()",
            ".collect::<",
            "String::with_capacity",
            "Vec::with_capacity",
            ".to_owned()",
            ".clone()",
        ],
        hot_calls: &[
            ".split(",
            ".replace(",
            ".contains(",
            ".find(",
            ".starts_with(",
            ".ends_with(",
            ".trim(",
            ".to_ascii_lowercase()",
            ".as_bytes()",
        ],
        branches: CLINE_BRANCHES,
        char_literals: true,
    },
    clike("c", "C", &["c", "h"], &[GENERIC_FN]),
    clike(
        "cpp",
        "C++",
        &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        &[GENERIC_FN],
    ),
    clike("csharp", "C#", &["cs"], &[GENERIC_FN]),
    clike(
        "go",
        "Go",
        &["go"],
        &[
            r"(?m)^\s*func\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)",
            GENERIC_FN,
        ],
    ),
    clike("java", "Java", &["java"], &[GENERIC_FN]),
    clike(
        "kotlin",
        "Kotlin",
        &["kt", "kts"],
        &[
            r"(?m)^\s*(?:public\s+|private\s+|internal\s+|protected\s+|open\s+|override\s+|suspend\s+|inline\s+)*fun\s+([A-Za-z_]\w*)",
            GENERIC_FN,
        ],
    ),
    clike(
        "swift",
        "Swift",
        &["swift"],
        &[
            r"(?m)^\s*(?:public\s+|private\s+|internal\s+|fileprivate\s+|open\s+|static\s+|class\s+|override\s+|mutating\s+)*func\s+([A-Za-z_]\w*)",
            GENERIC_FN,
        ],
    ),
    clike(
        "javascript",
        "JavaScript",
        &["js", "mjs", "cjs", "jsx"],
        &[
            r"(?m)^\s*(?:export\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][\w$]*)",
            r"(?m)^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?(?:function\b|\()",
            GENERIC_FN,
        ],
    ),
    clike(
        "typescript",
        "TypeScript",
        &["ts", "tsx", "mts"],
        &[
            r"(?m)^\s*(?:export\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][\w$]*)",
            r"(?m)^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*[:=][^=]*=\s*(?:async\s*)?(?:function\b|\()",
            GENERIC_FN,
        ],
    ),
    clike("dart", "Dart", &["dart"], &[GENERIC_FN]),
    clike(
        "scala",
        "Scala",
        &["scala", "sc"],
        &[
            r"(?m)^\s*(?:override\s+|private\s+|protected\s+|final\s+|implicit\s+)*def\s+([A-Za-z_]\w*)",
            GENERIC_FN,
        ],
    ),
    clike("groovy", "Groovy", &["groovy", "gradle"], &[GENERIC_FN]),
    clike(
        "zig",
        "Zig",
        &["zig"],
        &[r"(?m)^\s*(?:pub\s+)?fn\s+([A-Za-z_]\w*)", GENERIC_FN],
    ),
    clike(
        "nim",
        "Nim",
        &["nim", "nims"],
        &[r"(?m)^\s*(?:proc|func|method)\s+([A-Za-z_]\w*)", GENERIC_FN],
    ),
    clike(
        "solidity",
        "Solidity",
        &["sol"],
        &[r"(?m)^\s*function\s+([A-Za-z_]\w*)", GENERIC_FN],
    ),
    clike("objectivec", "Objective-C", &["m", "mm"], &[GENERIC_FN]),
    clike("d", "D", &["d"], &[GENERIC_FN]),
    clike("haxe", "Haxe", &["hx"], &[GENERIC_FN]),
    clike("vala", "Vala", &["vala"], &[GENERIC_FN]),
    LangProfile {
        id: "python",
        name: "Python",
        extensions: &["py", "pyi", "pyw"],
        line_comments: &["#"],
        block_comment: None,
        strings: &[("\"", "\\"), ("'", "\\")],
        raw_strings: &["r\"", "r'", "f\"", "f'"],
        fn_patterns: &[
            r"(?m)^\s*(?:async\s+)?def\s+([A-Za-z_]\w*)",
            r"(?m)^\s*class\s+([A-Za-z_]\w*)",
        ],
        body: BodyStyle::Indent,
        loops: &["for ", "while "],
        alloc_calls: &[
            "list(",
            "dict(",
            "set(",
            "tuple(",
            "[]",
            "append(",
            "extend(",
            "join(",
            "copy(",
            "deepcopy(",
            "str(",
            "int(",
            "float(",
        ],
        hot_calls: &[
            ".split(",
            ".replace(",
            ".strip(",
            ".lower(",
            ".upper(",
            ".startswith(",
            ".endswith(",
            ".find(",
            ".index(",
            ".count(",
        ],
        branches: &["if ", "elif ", "else", "match ", "case "],
        char_literals: false,
    },
    LangProfile {
        id: "ruby",
        name: "Ruby",
        extensions: &["rb", "rake"],
        line_comments: &["#"],
        block_comment: None,
        strings: &[("\"", "\\"), ("'", "\\")],
        raw_strings: &[],
        fn_patterns: &[
            r"(?m)^\s*def\s+(?:self\.)?([A-Za-z_]\w*[!?=]?)",
            r"(?m)^\s*([A-Za-z_]\w*)\s*=\s*(?:lambda|proc)\b",
        ],
        body: BodyStyle::End,
        loops: &["while ", "until ", ".each", "loop do"],
        alloc_calls: &["new(", "map(", "select(", "collect(", "+", "<<"],
        hot_calls: &[
            ".gsub(",
            ".sub(",
            ".split(",
            ".include?",
            ".start_with?",
            ".end_with?",
            ".match?",
            ".downcase",
            ".upcase",
            ".strip",
            ".index(",
        ],
        branches: &["if ", "unless ", "elsif ", "case ", "when "],
        char_literals: false,
    },
    LangProfile {
        id: "lua",
        name: "Lua",
        extensions: &["lua"],
        line_comments: &["--"],
        block_comment: Some(("--[[", "]]")),
        strings: &[("\"", "\\"), ("'", "\\")],
        raw_strings: &[],
        fn_patterns: &[
            r"(?m)^\s*(?:local\s+)?function\s+([A-Za-z_][\w.:]*)",
            r"(?m)^\s*(?:local\s+)?([A-Za-z_]\w*)\s*=\s*function\b",
        ],
        body: BodyStyle::End,
        loops: &["for ", "while ", "repeat "],
        alloc_calls: &["{", "table.insert", "string.rep", ".."],
        hot_calls: &[
            "string.find",
            "string.gsub",
            "string.sub",
            "table.concat",
            "string.match",
            "#",
        ],
        branches: &["if ", "elseif ", "else", "repeat "],
        char_literals: false,
    },
    LangProfile {
        id: "shell",
        name: "Shell",
        extensions: &["sh", "bash", "zsh", "ksh"],
        line_comments: &["#"],
        block_comment: None,
        strings: &[("\"", "\\"), ("'", "\\")],
        raw_strings: &[],
        fn_patterns: &[r"(?m)^\s*(?:function\s+)?([A-Za-z_]\w*)\s*\(\)\s*\{"],
        body: BodyStyle::Braces,
        loops: &["for ", "while ", "until "],
        alloc_calls: &["$(", "`", "cat ", "sed "],
        hot_calls: &["grep ", "sed ", "awk ", "tr ", "cut ", "wc "],
        branches: &["if ", "elif ", "else", "case ", "esac"],
        char_literals: false,
    },
    LangProfile {
        id: "powershell",
        name: "PowerShell",
        extensions: &["ps1", "psm1", "psd1"],
        line_comments: &["#"],
        block_comment: Some(("<#", "#>")),
        strings: &[("\"", "`"), ("'", "'")],
        raw_strings: &[],
        fn_patterns: &[
            r"(?m)^\s*function\s+([A-Za-z_][\w:-]*)",
            r"(?m)^\s*filter\s+([A-Za-z_][\w:-]*)",
        ],
        body: BodyStyle::Braces,
        loops: &["foreach ", "for ", "while ", "do "],
        alloc_calls: &["@(", "New-Object", "+=", "Join-Path"],
        hot_calls: &[
            "-match",
            "-replace",
            "-split",
            "Select-String",
            "Where-Object",
        ],
        branches: &["if ", "elseif ", "else", "switch "],
        char_literals: false,
    },
    LangProfile {
        id: "haskell",
        name: "Haskell",
        extensions: &["hs", "lhs"],
        line_comments: &["--"],
        block_comment: Some(("{-", "-}")),
        strings: &[("\"", "\\")],
        raw_strings: &[],
        fn_patterns: &[r"(?m)^([A-Za-z_]\w*)\s*(?::[^=\n]+)?="],
        body: BodyStyle::Expression,
        loops: &["map ", "fold", "filter ", "replicate", "iterate "],
        alloc_calls: &["++", "concatMap", "replicate"],
        hot_calls: &["length", "elem", "foldl'", "zipWith"],
        branches: &["if ", "case ", "of "],
        char_literals: false,
    },
    LangProfile {
        id: "ocaml",
        name: "OCaml",
        extensions: &["ml", "mli"],
        line_comments: &[],
        block_comment: Some(("(*", "*)")),
        strings: &[("\"", "\\")],
        raw_strings: &[],
        fn_patterns: &[r"(?m)^\s*let\s+(?:rec\s+)?([A-Za-z_]\w*)\s+[^=\n]*="],
        body: BodyStyle::Expression,
        loops: &["for ", "while ", "List.map", "List.fold"],
        alloc_calls: &["::", "List.map", "Array.make"],
        hot_calls: &["List.length", "List.iter", "String.length"],
        branches: &["if ", "match ", "| "],
        char_literals: false,
    },
    LangProfile {
        id: "erlang",
        name: "Erlang",
        extensions: &["erl", "hrl"],
        line_comments: &["%"],
        block_comment: None,
        strings: &[("\"", "\\")],
        raw_strings: &[],
        fn_patterns: &[r"(?m)^([A-Za-z_]\w*)\s*\([^)]*\)\s*->"],
        body: BodyStyle::Expression,
        loops: &["lists:foreach", "lists:map", "recursive"],
        alloc_calls: &["++", "lists:append"],
        hot_calls: &["lists:map", "lists:foreach", "lists:seq"],
        branches: &["case ", "if ", "receive "],
        char_literals: false,
    },
    LangProfile {
        id: "elixir",
        name: "Elixir",
        extensions: &["ex", "exs"],
        line_comments: &["#"],
        block_comment: None,
        strings: &[("\"", "\\")],
        raw_strings: &["\"\"\"", "'''"],
        fn_patterns: &[
            r"(?m)^\s*def(?:p)?\s+([A-Za-z_]\w*[!?]?)",
            r"(?m)^\s*defmacro\s+([A-Za-z_]\w*)",
        ],
        body: BodyStyle::End,
        loops: &["for ", "Enum.", "Stream."],
        alloc_calls: &["++", "Enum.map", "Enum.reduce"],
        hot_calls: &["Enum.map", "Enum.filter", "String.", "Map."],
        branches: &["if ", "else", "case ", "cond "],
        char_literals: false,
    },
    LangProfile {
        id: "julia",
        name: "Julia",
        extensions: &["jl"],
        line_comments: &["#"],
        block_comment: Some(("#=", "=#")),
        strings: &[("\"", "\\")],
        raw_strings: &["raw\""],
        fn_patterns: &[r"(?m)^\s*function\s+([A-Za-z_]\w*)"],
        body: BodyStyle::End,
        loops: &["for ", "while ", "eachindex", "broadcast"],
        alloc_calls: &["zeros(", "ones(", "collect(", "vcat"],
        hot_calls: &["sum(", "map(", "filter(", "dot(", "muladd"],
        branches: &["if ", "elseif ", "else"],
        char_literals: false,
    },
    LangProfile {
        id: "r",
        name: "R",
        extensions: &["r", "R"],
        line_comments: &["#"],
        block_comment: None,
        strings: &[("\"", "\\"), ("'", "\\")],
        raw_strings: &[],
        fn_patterns: &[r"(?m)^\s*([A-Za-z_.]\w*)\s*(?:<-|=)\s*function\b"],
        body: BodyStyle::Expression,
        loops: &["for ", "while ", "sapply", "lapply", "vapply"],
        alloc_calls: &["c(", "rep(", "matrix("],
        hot_calls: &["paste", "gsub", "grepl", "strsplit"],
        branches: &["if ", "else", "switch("],
        char_literals: false,
    },
    LangProfile {
        id: "fortran",
        name: "Fortran",
        extensions: &["f90", "f95", "f03", "for"],
        line_comments: &["!"],
        block_comment: None,
        strings: &[("\"", "\\"), ("'", "\\")],
        raw_strings: &[],
        fn_patterns: &[
            r"(?mi)^\s*(?:recursive\s+)?(?:pure\s+)?(?:elemental\s+)?function\s+([A-Za-z_]\w*)",
            r"(?mi)^\s*subroutine\s+([A-Za-z_]\w*)",
        ],
        body: BodyStyle::End,
        loops: &["do ", "do while", "forall"],
        alloc_calls: &["allocate(", "allocatable"],
        hot_calls: &["dot_product", "sum(", "maxval", "minval"],
        branches: &["if ", "else", "select case", "case "],
        char_literals: false,
    },
    LangProfile {
        id: "matlab",
        name: "MATLAB",
        extensions: &["m"],
        line_comments: &["%"],
        block_comment: Some(("%{", "%}")),
        strings: &[("\"", "\\"), ("'", "\\")],
        raw_strings: &[],
        fn_patterns: &[r"(?m)^\s*function\s+(?:\[[^\]]*\]|[\w]+)\s*=\s*([A-Za-z_]\w*)"],
        body: BodyStyle::End,
        loops: &["for ", "while "],
        alloc_calls: &["zeros(", "ones(", "cell(", "repmat"],
        hot_calls: &["sum(", "dot(", "vectorize", "arrayfun"],
        branches: &["if ", "elseif ", "else", "switch ", "case "],
        char_literals: false,
    },
    LangProfile {
        id: "perl",
        name: "Perl",
        extensions: &["pl", "pm", "t"],
        line_comments: &["#"],
        block_comment: None,
        strings: &[("\"", "\\"), ("'", "\\")],
        raw_strings: &["qq", "qw"],
        fn_patterns: &[r"(?m)^\s*sub\s+([A-Za-z_]\w*)"],
        body: BodyStyle::Braces,
        loops: &["for ", "foreach ", "while ", "map ", "grep "],
        alloc_calls: &["push ", "@", "map "],
        hot_calls: &["substr", "index", "sprintf", "join"],
        branches: &["if ", "elsif ", "else", "unless "],
        char_literals: false,
    },
    LangProfile {
        id: "sql",
        name: "SQL",
        extensions: &["sql", "ddl", "dml"],
        line_comments: &["--"],
        block_comment: Some(("/*", "*/")),
        strings: &[("'", "''")],
        raw_strings: &[],
        fn_patterns: &[
            r"(?mi)^\s*create\s+(?:or\s+replace\s+)?(?:function|procedure)\s+([A-Za-z_][\w.]*)",
        ],
        body: BodyStyle::Expression,
        loops: &["for ", "while "],
        alloc_calls: &["insert ", "create table", "temp table"],
        hot_calls: &["select ", "join ", "where "],
        branches: &["case ", "when ", "where "],
        char_literals: false,
    },
    LangProfile {
        id: "php",
        name: "PHP",
        extensions: &["php", "phtml"],
        line_comments: &["//", "#"],
        block_comment: Some(("/*", "*/")),
        strings: &[("\"", "\\"), ("'", "\\")],
        raw_strings: &[],
        fn_patterns: &[
            r"(?m)^\s*(?:public\s+|private\s+|protected\s+|static\s+|final\s+|abstract\s+)*function\s+([A-Za-z_]\w*)",
        ],
        body: BodyStyle::Braces,
        loops: &["for ", "foreach ", "while ", "do "],
        alloc_calls: &["new ", "array_merge", "[]", ".="],
        hot_calls: &[
            "str_replace",
            "explode",
            "implode",
            "substr",
            "strpos",
            "preg_match",
            "count",
        ],
        branches: &["if ", "elseif ", "else", "switch ", "match "],
        char_literals: false,
    },
    LangProfile {
        id: "objectivecxx",
        name: "Objective-C++",
        extensions: &["mmx"],
        line_comments: CLINE_COMMENTS,
        block_comment: Some(CLINE_BLOCK),
        strings: CLINE_STRINGS,
        raw_strings: &[],
        fn_patterns: &[GENERIC_FN],
        body: BodyStyle::Braces,
        loops: CLINE_LOOPS,
        alloc_calls: CLINE_ALLOC,
        hot_calls: &[],
        branches: CLINE_BRANCHES,
        char_literals: false,
    },
];

/// Langage correspondant à une extension de fichier (sans le point).
pub fn by_extension(ext: &str) -> Option<&'static LangProfile> {
    let ext = ext.to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|l| l.extensions.iter().any(|e| e.to_ascii_lowercase() == ext))
}

/// Tous les langages, triés par identifiant.
pub fn all() -> Vec<&'static LangProfile> {
    let mut v: Vec<&'static LangProfile> = LANGUAGES.iter().collect();
    v.sort_by_key(|l| l.id);
    v
}

/// Nombre de langages supportés.
pub fn count() -> usize {
    LANGUAGES.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_resolve_to_a_language() {
        assert_eq!(by_extension("rs").map(|l| l.id), Some("rust"));
        assert_eq!(by_extension("py").map(|l| l.id), Some("python"));
        assert_eq!(by_extension("go").map(|l| l.id), Some("go"));
        assert_eq!(by_extension("RS").map(|l| l.id), Some("rust"));
        assert!(by_extension("unknownext").is_none());
    }

    #[test]
    fn registry_covers_many_languages() {
        assert!(count() >= 30, "seulement {} langages", count());
        let mut ids: Vec<&str> = LANGUAGES.iter().map(|l| l.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "identifiant de langage dupliqué");
    }

    #[test]
    fn every_language_has_comments_and_definitions() {
        for l in LANGUAGES {
            assert!(
                !l.line_comments.is_empty() || l.block_comment.is_some(),
                "{} : aucun commentaire déclaré",
                l.id
            );
            assert!(
                !l.fn_patterns.is_empty(),
                "{} : aucun motif de définition",
                l.id
            );
        }
    }
}
