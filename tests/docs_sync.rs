//! Checks that the docs agree with the code. Everything here reads files as
//! text: no GPUI, no window, runs in milliseconds.
//!
//! - Every `KeyBinding::new("keys", Action, …)` in `src/` is listed in the
//!   README key table, and every key in that table is really bound.
//! - The `--help` text (`HELP` in `src/`) names at least one key for every
//!   bound action, and every key it names is really bound.
//! - Relative links in the Markdown docs point to files that exist, and
//!   `#anchors` to headings that exist.
//! - Repo paths (`src/viewer.rs`, `crates/mdview-core/src/markdown.rs`,
//!   `patches/…`) and Rust file names (`render.rs`) mentioned in the docs
//!   exist.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Component, Path, PathBuf},
};

/// Bindings that are known to be missing from a doc. Fix the doc and remove
/// the entry; the test fails if an entry here is no longer needed.
const KNOWN_GAPS: &[(&str, &str)] = &[];

/// Markdown files whose links and paths are checked.
fn doc_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = [
        "README.md",
        "AGENTS.md",
        "CLAUDE.md",
        "crates/gpui/MDVIEW.md",
    ]
    .iter()
    .map(|f| root().join(f))
    .filter(|p| p.exists())
    .collect();
    for dir in ["docs", "crates/mdview-core", "examples"] {
        walk(&root().join(dir), &mut |p| {
            if p.extension().is_some_and(|e| e == "md") && !is_test_input(p) {
                files.push(p.to_path_buf());
            }
        });
    }
    files.sort();
    files
}

/// Markdown that is test *input* (parser fixtures, vendored spec data,
/// snapshots), not documentation: its links are broken on purpose. READMEs
/// that describe those directories are still checked.
fn is_test_input(p: &Path) -> bool {
    let rel = p.strip_prefix(root()).unwrap_or(p);
    let in_dir = |d: &str| rel.components().any(|c| c.as_os_str() == d);
    let readme = p.file_name().is_some_and(|n| n == "README.md");
    in_dir("fixtures") || in_dir("snapshots") || (in_dir("data") && !readme)
}

/// The directory of the closest `Cargo.toml` above `file` (or the repo root):
/// docs inside a crate may write paths relative to that crate.
fn crate_root(file: &Path) -> PathBuf {
    file.ancestors()
        .skip(1)
        .find(|d| d.join("Cargo.toml").exists())
        .unwrap_or(&root())
        .to_path_buf()
}

// ---------------------------------------------------------------- key bindings

#[derive(Debug)]
struct Binding {
    keys: String,
    action: String,
    at: String,
}

#[test]
fn readme_key_table_matches_key_bindings() {
    let bindings = key_bindings();
    let readme = read("README.md");
    let table = readme_key_table(&readme);
    let mut problems = Vec::new();

    let mut gaps_used = BTreeSet::new();
    for b in &bindings {
        let forms = human_forms(&b.keys);
        if forms.iter().any(|f| table.contains(f)) {
            continue;
        }
        if KNOWN_GAPS.contains(&("README.md", b.keys.as_str())) {
            gaps_used.insert(b.keys.clone());
            continue;
        }
        problems.push(format!(
            "{}: `{}` ({}) is bound but missing from the README key table. Add a row with `{}`.",
            b.at, b.keys, b.action, forms[0]
        ));
    }
    let all_forms: BTreeSet<String> = bindings.iter().flat_map(|b| human_forms(&b.keys)).collect();
    for key in &table {
        if !all_forms.contains(key) {
            problems.push(format!(
                "README.md: the key table lists `{key}`, but no KeyBinding in src/ produces it. \
                 Remove it, or bind it (known forms: {}).",
                join(&all_forms)
            ));
        }
    }
    for (doc, keys) in KNOWN_GAPS {
        if *doc == "README.md" && !gaps_used.contains(*keys) {
            problems.push(format!(
                "tests/docs_sync.rs: KNOWN_GAPS entry ({doc}, {keys}) isn't needed any more. Remove it."
            ));
        }
    }
    report("README key table and key bindings disagree", problems);
}

#[test]
fn help_text_matches_key_bindings() {
    let bindings = key_bindings();
    let (help, help_at) = help_text();
    let tokens = help_key_tokens(&help);
    let mut problems = Vec::new();

    // HELP is a summary: one key per action is enough.
    let mut by_action: BTreeMap<&str, Vec<&Binding>> = BTreeMap::new();
    for b in &bindings {
        by_action.entry(&b.action).or_default().push(b);
    }
    for (action, bs) in &by_action {
        let shown = bs
            .iter()
            .any(|b| human_forms(&b.keys).iter().any(|f| tokens.contains(f)));
        if !shown {
            let keys: Vec<String> = bs.iter().map(|b| human_forms(&b.keys)[0].clone()).collect();
            problems.push(format!(
                "{help_at}: HELP doesn't mention any key for the {action} action. Add one of: {}.",
                keys.join(", ")
            ));
        }
    }
    let all_forms: BTreeSet<String> = bindings.iter().flat_map(|b| human_forms(&b.keys)).collect();
    for t in &tokens {
        if looks_like_key(t) && !all_forms.contains(t) {
            problems.push(format!(
                "{help_at}: HELP mentions `{t}`, but no KeyBinding in src/ produces it."
            ));
        }
    }
    report("--help text and key bindings disagree", problems);
}

/// Every `KeyBinding::new("…", Action, …)` in `src/`.
fn key_bindings() -> Vec<Binding> {
    let mut out = Vec::new();
    for file in rust_files(&root().join("src")) {
        let text = fs::read_to_string(&file).unwrap();
        let rel = rel(&file);
        let needle = "KeyBinding::new(";
        let mut from = 0;
        while let Some(i) = text[from..].find(needle) {
            let start = from + i;
            from = start + needle.len();
            let line = text[..start].matches('\n').count() + 1;
            let at = format!("{rel}:{line}");
            let rest = text[from..].trim_start();
            let parsed = rest.strip_prefix('"').and_then(|r| {
                let end = r.find('"')?;
                let keys = &r[..end];
                let after = r[end + 1..].trim_start().strip_prefix(',')?.trim_start();
                let ident: String = after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                    .collect();
                let action = ident.rsplit("::").next()?.to_string();
                (!action.is_empty()).then(|| (keys.to_string(), action))
            });
            match parsed {
                Some((keys, action)) => out.push(Binding { keys, action, at }),
                None => panic!(
                    "{at}: can't parse this KeyBinding. tests/docs_sync.rs expects the literal form \
                     KeyBinding::new(\"keys\", Action, context) so it can check the docs."
                ),
            }
        }
    }
    assert!(
        out.len() >= 5,
        "found only {} KeyBinding::new calls in src/: did the bindings move or change form? \
         Update key_bindings() in tests/docs_sync.rs.",
        out.len()
    );
    out
}

/// How a GPUI keystroke like `ctrl-shift-tab` is written for people. The
/// first form is the preferred one.
fn human_forms(keystrokes: &str) -> Vec<String> {
    // A binding can be a sequence of keystrokes separated by spaces.
    let per_stroke: Vec<Vec<String>> = keystrokes.split(' ').map(human_forms_one).collect();
    per_stroke.iter().fold(vec![String::new()], |acc, forms| {
        acc.iter()
            .flat_map(|a| {
                forms.iter().map(move |f| {
                    if a.is_empty() {
                        f.clone()
                    } else {
                        format!("{a} {f}")
                    }
                })
            })
            .collect()
    })
}

fn human_forms_one(stroke: &str) -> Vec<String> {
    let parts: Vec<&str> = if let Some(prefix) = stroke.strip_suffix("--") {
        // `ctrl--` is ctrl + minus.
        prefix.split('-').chain(["-"]).collect()
    } else {
        stroke.split('-').collect()
    };
    let (key, mods) = parts.split_last().unwrap();
    let mods: Vec<&str> = mods
        .iter()
        .map(|m| match *m {
            "ctrl" => "Ctrl",
            "shift" => "Shift",
            "alt" => "Alt",
            "cmd" | "super" | "win" | "platform" => "Super",
            "fn" => "Fn",
            other => panic!("unknown modifier `{other}` in key binding `{stroke}`"),
        })
        .collect();
    let named = match *key {
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        "pageup" => "PgUp".to_string(),
        "pagedown" => "PgDn".to_string(),
        "space" => "Space".to_string(),
        "tab" => "Tab".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "enter" => "Enter".to_string(),
        "escape" => "Esc".to_string(),
        "backspace" => "Backspace".to_string(),
        "delete" => "Del".to_string(),
        k if k.chars().count() == 1 && mods.is_empty() => k.to_string(),
        k => {
            let mut c = k.chars();
            let first = c.next().unwrap().to_uppercase().to_string();
            first + c.as_str()
        }
    };
    let mut forms = Vec::new();
    if mods.is_empty() {
        forms.push(named.clone());
    } else {
        forms.push(format!("{}+{named}", mods.join("+")));
    }
    // `shift-g` is usually written `G`.
    if mods == ["Shift"] && key.len() == 1 && key.chars().all(|c| c.is_ascii_alphabetic()) {
        forms.insert(0, key.to_uppercase());
    }
    forms
}

fn looks_like_key(token: &str) -> bool {
    const NAMED: &[&str] = &[
        "↑",
        "↓",
        "←",
        "→",
        "PgUp",
        "PgDn",
        "Space",
        "Tab",
        "Home",
        "End",
        "Enter",
        "Esc",
        "Backspace",
        "Del",
    ];
    let first = token.split('+').next().unwrap_or("");
    (token.contains('+') && ["Ctrl", "Shift", "Alt", "Super", "Fn"].contains(&first))
        || NAMED.contains(&token)
        || (token.len() >= 2
            && token.starts_with('F')
            && token[1..].chars().all(|c| c.is_ascii_digit()))
}

/// The keys in the first column of the README table whose header has "Key".
fn readme_key_table(readme: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let mut in_table = false;
    for line in readme.lines() {
        let l = line.trim();
        if !l.starts_with('|') {
            if in_table {
                break;
            }
            continue;
        }
        let cells: Vec<&str> = l.trim_matches('|').split('|').map(str::trim).collect();
        if !in_table {
            in_table = cells.first().is_some_and(|c| c.eq_ignore_ascii_case("key"));
            continue;
        }
        for span in code_spans(cells[0]) {
            keys.extend(span.split_whitespace().map(str::to_string));
        }
    }
    assert!(
        !keys.is_empty(),
        "README.md: couldn't find the key table (a Markdown table whose first header cell is \
         `Key`, with the keys in `backticks`)"
    );
    keys
}

/// `HELP` from `src/`, unescaped, and where it was found.
fn help_text() -> (String, String) {
    for file in rust_files(&root().join("src")) {
        let text = fs::read_to_string(&file).unwrap();
        let Some(i) = text.find("const HELP: &str = \"") else {
            continue;
        };
        let line = text[..i].matches('\n').count() + 1;
        let body = &text[i + "const HELP: &str = \"".len()..];
        let mut out = String::new();
        let mut chars = body.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' => return (out, format!("{}:{line}", rel(&file))),
                '\\' => match chars.next() {
                    Some('\n') => {
                        while chars.peek().is_some_and(|c| c.is_whitespace()) {
                            chars.next();
                        }
                    }
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => break,
                },
                c => out.push(c),
            }
        }
    }
    panic!("couldn't find `const HELP: &str = \"…\"` in src/ (tests/docs_sync.rs reads it)");
}

fn help_key_tokens(help: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for t in help.split(|c: char| c.is_whitespace() || c == '·' || c == '/' || c == ',') {
        if t.is_empty() {
            continue;
        }
        // "↑↓" is two keys.
        if t.chars().all(|c| "↑↓←→".contains(c)) {
            out.extend(t.chars().map(String::from));
        } else {
            out.insert(t.to_string());
        }
    }
    out
}

// ----------------------------------------------------------------- doc links

#[test]
fn relative_links_in_docs_resolve() {
    let mut problems = Vec::new();
    let mut slug_cache: HashMap<PathBuf, BTreeSet<String>> = HashMap::new();
    for file in doc_files() {
        let text = fs::read_to_string(&file).unwrap();
        let dir = file.parent().unwrap();
        for (line, target) in links(&text) {
            let at = format!("{}:{line}", rel(&file));
            if is_external(&target) {
                continue;
            }
            let (path, anchor) = match target.split_once('#') {
                Some((p, a)) => (p, Some(a)),
                None => (target.as_str(), None),
            };
            let resolved = if path.is_empty() {
                file.clone()
            } else {
                normalize(&dir.join(percent_decode(path)))
            };
            if !resolved.starts_with(root()) {
                problems.push(format!("{at}: link `{target}` points outside the repo"));
                continue;
            }
            if !resolved.exists() {
                problems.push(format!(
                    "{at}: link `{target}` is broken: {} doesn't exist",
                    rel(&resolved)
                ));
                continue;
            }
            let Some(anchor) = anchor.filter(|a| !a.is_empty()) else {
                continue;
            };
            if resolved.extension().is_none_or(|e| e != "md") {
                continue; // anchors into code files (#L10) aren't checked
            }
            let slugs = slug_cache
                .entry(resolved.clone())
                .or_insert_with(|| heading_slugs(&fs::read_to_string(&resolved).unwrap()));
            if !slugs.contains(&anchor.to_lowercase()) {
                problems.push(format!(
                    "{at}: link `{target}`: {} has no heading with the anchor `#{anchor}`. \
                     Existing anchors: {}",
                    rel(&resolved),
                    join(slugs.iter())
                ));
            }
        }
    }
    report("broken links in the docs", problems);
}

#[test]
fn source_paths_in_docs_exist() {
    let files = repo_files();
    let rs_files: Vec<&String> = files.iter().filter(|f| f.ends_with(".rs")).collect();
    let top_dirs = [
        "src/",
        "crates/",
        "docs/",
        "patches/",
        "scripts/",
        "tests/",
        "examples/",
        "assets/",
        ".github/",
    ];
    let mut problems = Vec::new();
    for file in doc_files() {
        let text = fs::read_to_string(&file).unwrap();
        // Tree listings in code blocks: an indented path is relative to the
        // directory on the closest less-indented line above it.
        let mut fences = Fences::default();
        let mut tree: Vec<(usize, String)> = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let at = format!("{}:{}", rel(&file), n + 1);
            let was_inside = fences.inside();
            fences.skip(line);
            let in_fence = fences.inside();
            if was_inside != in_fence {
                tree.clear(); // a fence line
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            let mut prefix = String::new();
            if in_fence {
                while tree.last().is_some_and(|(i, _)| *i >= indent) {
                    tree.pop();
                }
                prefix = tree.last().map(|(_, d)| d.clone()).unwrap_or_default();
                let first = line.split_whitespace().next().unwrap_or("");
                if first.ends_with('/') && !first.starts_with('/') {
                    tree.push((indent, format!("{prefix}{first}")));
                }
            }
            for token in path_tokens(line) {
                let t = token.trim_end_matches('.');
                if top_dirs.iter().any(|d| t.starts_with(d)) {
                    let full = format!("{prefix}{t}");
                    if !root().join(&full).exists()
                        && !root().join(t).exists()
                        && !crate_root(&file).join(t).exists()
                    {
                        problems.push(format!("{at}: `{t}` doesn't exist"));
                    }
                } else if t.ends_with(".rs") {
                    // `render.rs` or `platform/blade/blade_renderer.rs`: some
                    // Rust file in the repo must end with it.
                    let suffix = format!("/{t}");
                    if !rs_files.iter().any(|f| f.ends_with(&suffix) || **f == t) {
                        problems.push(format!(
                            "{at}: `{t}` doesn't match any Rust file in the repo. \
                             Was it renamed or moved?"
                        ));
                    }
                }
            }
        }
    }
    report("paths in the docs that don't exist", problems);
}

/// Maximal runs of path characters that look like repo paths or Rust files,
/// skipping absolute paths, URLs and globs.
fn path_tokens(line: &str) -> Vec<&str> {
    let is_path_char = |c: char| c.is_ascii_alphanumeric() || "_./-".contains(c);
    let mut out = Vec::new();
    let bytes: Vec<(usize, char)> = line.char_indices().collect();
    let mut i = 0;
    while i < bytes.len() {
        if !is_path_char(bytes[i].1) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_path_char(bytes[i].1) {
            i += 1;
        }
        let s = bytes[start].0;
        let e = bytes.get(i).map_or(line.len(), |b| b.0);
        let token = &line[s..e];
        let before = start.checked_sub(1).map(|j| bytes[j].1);
        let after = bytes.get(i).map(|b| b.1);
        let skip = token.starts_with('/')
            || token.starts_with("..")
            || matches!(before, Some('~' | '$' | ':' | '*' | '@' | '{' | '%' | '\\'))
            || matches!(after, Some('*' | '{' | '$' | '<'));
        if !skip && (token.contains('/') || token.ends_with(".rs")) {
            out.push(token);
        }
    }
    out
}

/// (line number, target) of every Markdown/HTML link outside code.
fn links(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut fences = Fences::default();
    for (n, line) in text.lines().enumerate() {
        if fences.skip(line) {
            continue;
        }
        let line = strip_code_spans(line);
        // [text](target "title") and ![alt](target)
        let mut rest = line.as_str();
        while let Some(i) = rest.find("](") {
            let after = &rest[i + 2..];
            let end = after.find(')').unwrap_or(after.len());
            let raw = after[..end].trim();
            let target = raw
                .strip_prefix('<')
                .and_then(|r| r.split_once('>').map(|x| x.0))
                .unwrap_or_else(|| raw.split_whitespace().next().unwrap_or(""));
            if !target.is_empty() {
                out.push((n + 1, target.to_string()));
            }
            rest = &after[end.min(after.len())..];
        }
        // [ref]: target
        let t = line.trim_start();
        if t.starts_with('[')
            && !t.starts_with("[^") // footnote
            && let Some(i) = t.find("]:")
            && !t[..i].contains('(')
            && let Some(target) = t[i + 2..].split_whitespace().next()
        {
            out.push((n + 1, target.to_string()));
        }
        // <img src="…"> / <a href="…">
        for attr in ["src=\"", "href=\""] {
            let mut rest = line.as_str();
            while let Some(i) = rest.find(attr) {
                let after = &rest[i + attr.len()..];
                let end = after.find('"').unwrap_or(after.len());
                out.push((n + 1, after[..end].to_string()));
                rest = &after[end..];
            }
        }
    }
    out
}

fn is_external(target: &str) -> bool {
    let scheme = target.split(':').next().unwrap_or("");
    target.contains(':')
        && !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
}

/// GitHub's anchors for the headings of a Markdown file.
fn heading_slugs(text: &str) -> BTreeSet<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = BTreeSet::new();
    let mut fences = Fences::default();
    let mut prev = "";
    for line in text.lines() {
        if fences.skip(line) {
            prev = "";
            continue;
        }
        let t = line.trim_end();
        let heading = if let Some(h) = atx_heading(t) {
            Some(h.to_string())
        } else if !prev.trim().is_empty()
            && !prev.trim_start().starts_with(['|', '-', '*', '>', '#'])
            && t.len() >= 2
            && (t.chars().all(|c| c == '=') || t.chars().all(|c| c == '-'))
        {
            Some(prev.trim().to_string())
        } else {
            None
        };
        prev = line;
        let Some(h) = heading else { continue };
        let base = slugify(&h);
        let n = seen.entry(base.clone()).or_insert(0);
        out.insert(if *n == 0 {
            base.clone()
        } else {
            format!("{base}-{n}")
        });
        *n += 1;
    }
    out
}

fn atx_heading(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if line.len() - t.len() > 3 {
        return None;
    }
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &t[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some(rest.trim().trim_end_matches('#').trim())
}

/// github-slugger: drop the link targets and punctuation, lowercase, and turn
/// each space into a hyphen.
fn slugify(heading: &str) -> String {
    let mut text = String::new();
    let mut rest = heading;
    while let Some(i) = rest.find("](") {
        text.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        rest = after.find(')').map_or("", |e| &after[e + 1..]);
    }
    text.push_str(rest);
    text.to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

// ------------------------------------------------------------------- helpers

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel_path: &str) -> String {
    fs::read_to_string(root().join(rel_path))
        .unwrap_or_else(|e| panic!("can't read {rel_path}: {e}"))
}

fn rel(p: &Path) -> String {
    // Documentation paths use '/', independently of the host filesystem.
    p.strip_prefix(root())
        .unwrap_or(p)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy();
        if name == "target" || name == ".git" {
            continue;
        }
        if p.is_dir() {
            walk(&p, f);
        } else {
            f(&p);
        }
    }
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(dir, &mut |p| {
        if p.extension().is_some_and(|e| e == "rs") {
            out.push(p.to_path_buf());
        }
    });
    out
}

/// All files in the repo (minus target/ and .git), as repo-relative paths.
fn repo_files() -> Vec<String> {
    let mut out = Vec::new();
    walk(&root(), &mut |p| out.push(rel(p)));
    out
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Tracks fenced code blocks (CommonMark rules, minus the indentation details).
#[derive(Default)]
struct Fences {
    open: Option<(char, usize)>,
}

impl Fences {
    /// Feeds the next line. True for fence lines and the lines between them.
    fn skip(&mut self, line: &str) -> bool {
        let t = line.trim_start();
        let indent = line.len() - t.len();
        let ch = t.chars().next().unwrap_or(' ');
        let n = if ch == '`' || ch == '~' {
            t.chars().take_while(|c| *c == ch).count()
        } else {
            0
        };
        match self.open {
            Some((c, len)) => {
                if ch == c && n >= len && t[n..].trim().is_empty() {
                    self.open = None;
                }
                true
            }
            // ```` ```x ```` at the start of a line is a code span, not a
            // fence: a backtick fence's info string can't contain backticks.
            None if indent <= 3 && n >= 3 && !(ch == '`' && t[n..].contains('`')) => {
                self.open = Some((ch, n));
                true
            }
            None => false,
        }
    }

    fn inside(&self) -> bool {
        self.open.is_some()
    }
}

/// Contents of the `code spans` in a line.
fn code_spans(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find('`') {
        let ticks = rest[i..].chars().take_while(|c| *c == '`').count();
        let fence = &rest[i..i + ticks];
        let after = &rest[i + ticks..];
        let Some(end) = after.find(fence) else { break };
        out.push(after[..end].trim());
        rest = &after[end + ticks..];
    }
    out
}

fn strip_code_spans(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find('`') {
        let ticks = rest[i..].chars().take_while(|c| *c == '`').count();
        let fence = &rest[i..i + ticks];
        let after = &rest[i + ticks..];
        let Some(end) = after.find(fence) else { break };
        out.push_str(&rest[..i]);
        rest = &after[end + ticks..];
    }
    out.push_str(rest);
    out
}

fn join<'a>(items: impl IntoIterator<Item = &'a String>) -> String {
    items
        .into_iter()
        .map(|s| format!("`{s}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn report(title: &str, problems: Vec<String>) {
    assert!(
        problems.is_empty(),
        "{title} ({} problem{}):\n  - {}\n",
        problems.len(),
        if problems.len() == 1 { "" } else { "s" },
        problems.join("\n  - ")
    );
}

// Self-tests for the parsers above, so a broken checker doesn't pass silently.

#[test]
fn checker_repo_paths_use_documentation_separators() {
    let path = root()
        .join("crates")
        .join("mdview-core")
        .join("src")
        .join("markdown.rs");
    assert_eq!(rel(&path), "crates/mdview-core/src/markdown.rs");
    let files = repo_files();
    assert!(files.iter().any(|file| file.ends_with("/markdown.rs")));
    assert!(
        !files
            .iter()
            .any(|file| file.ends_with("/missing-mdview-file.rs"))
    );
}

#[test]
fn checker_human_forms() {
    assert_eq!(human_forms("ctrl-shift-tab")[0], "Ctrl+Shift+Tab");
    assert_eq!(human_forms("ctrl-o")[0], "Ctrl+O");
    assert_eq!(human_forms("shift-g")[0], "G");
    assert_eq!(human_forms("pagedown")[0], "PgDn");
    assert_eq!(human_forms("f5")[0], "F5");
    assert_eq!(human_forms("k")[0], "k");
    assert_eq!(human_forms("ctrl-k ctrl-w")[0], "Ctrl+K Ctrl+W");
}

#[test]
fn checker_slugs_and_links() {
    let slugs = heading_slugs(
        "```` ```mermaid ```` is a code span\n# What the project is (and isn't)\n## `src/` layout\n```\n# not a heading\n```\n## Dup\n## Dup\nSetext\n---\n",
    );
    for s in [
        "what-the-project-is-and-isnt",
        "src-layout",
        "dup",
        "dup-1",
        "setext",
    ] {
        assert!(slugs.contains(s), "{s} missing from {slugs:?}");
    }
    assert!(!slugs.contains("not-a-heading"));
    let l = links("See [a](b.md#c) and `[x](not-a-link)` ![i](img.png \"t\")\n[r]: ref.md\n");
    let targets: Vec<&str> = l.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(targets, ["b.md#c", "img.png", "ref.md"]);
    assert!(is_external("https://example.com") && is_external("mailto:a@b"));
    assert!(!is_external("docs/a.md"));
    assert_eq!(
        path_tokens("see `src/main.rs`, ~/.cargo/src/x.rs, https://a/src/b and docs/*.md"),
        ["src/main.rs"]
    );
    assert_eq!(
        help_key_tokens("Ctrl+Tab / Ctrl+Shift+Tab · ↑↓ PgUp").len(),
        5
    );
}
