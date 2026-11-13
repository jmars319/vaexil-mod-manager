//! Windows command-line quoting, for Steam's `%command%` and the launch
//! options that wrap it. Copied from Modlist Launcher's `launch.rs`.

/// Quotes one argument so `CommandLineToArgvW` (and MSVC's runtime) parse it
/// back unchanged.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// Splits a command line the way `CommandLineToArgvW` does for arguments
/// after the program name. Used for tests and for Steam's `%command%`.
pub fn split_command_line(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            let start = i;
            while i < chars.len() && chars[i] == '\\' {
                i += 1;
            }
            let n = i - start;
            if i < chars.len() && chars[i] == '"' {
                cur.extend(std::iter::repeat_n('\\', n / 2));
                if n % 2 == 1 {
                    cur.push('"');
                    i += 1;
                }
            } else {
                cur.extend(std::iter::repeat_n('\\', n));
            }
            has_token = true;
            continue;
        }
        if c == '"' {
            if in_quotes && chars.get(i + 1) == Some(&'"') {
                cur.push('"');
                i += 2;
                continue;
            }
            in_quotes = !in_quotes;
            has_token = true;
        } else if (c == ' ' || c == '\t') && !in_quotes {
            if has_token {
                args.push(std::mem::take(&mut cur));
                has_token = false;
            }
        } else {
            cur.push(c);
            has_token = true;
        }
        i += 1;
    }
    if has_token {
        args.push(cur);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_round_trips() {
        let cases = [
            "plain",
            "with space",
            "",
            r"C:\Program Files (x86)\Steam\",
            r#"say "hi""#,
            r#"tricky \"quoted\" \\"#,
            "moshortcut://:The Witcher 3: Wild Hunt",
            "tab\there",
        ];
        let line = cases.iter().map(|c| quote_arg(c)).collect::<Vec<_>>().join(" ");
        assert_eq!(split_command_line(&line), cases);
    }

    #[test]
    fn split_handles_steam_command() {
        let line =
            r#""C:\Program Files (x86)\Steam\steamapps\common\Cyberpunk 2077\REDprelauncher.exe" -skipStartScreen"#;
        assert_eq!(
            split_command_line(line),
            vec![
                r"C:\Program Files (x86)\Steam\steamapps\common\Cyberpunk 2077\REDprelauncher.exe",
                "-skipStartScreen"
            ]
        );
    }
}
