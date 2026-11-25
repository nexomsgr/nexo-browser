//! CSV для импорта и экспорта паролей. Формат экспорта — как у Chrome/Brave: `name,url,username,password,note`.
//! Импорт понимает заголовки Chrome, Brave, Edge, Firefox, Bitwarden и похожие (по названиям столбцов).

use super::NewCredential;
use anyhow::{bail, Result};

const MAX_ROWS: usize = 100_000;

/// Разбор CSV по RFC 4180: кавычки, удвоенные кавычки, переводы строк внутри полей, BOM, CRLF.
pub fn parse(text: &str) -> Result<Vec<Vec<String>>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();

    let end_row = |row: &mut Vec<String>, field: &mut String, rows: &mut Vec<Vec<String>>| {
        row.push(std::mem::take(field));
        if !(row.len() == 1 && row[0].is_empty()) {
            rows.push(std::mem::take(row));
        } else {
            row.clear();
        }
    };

    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => in_quotes = false,
                _ => field.push(c),
            }
        } else {
            match c {
                '"' if field.is_empty() => in_quotes = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    end_row(&mut row, &mut field, &mut rows);
                }
                '\n' => end_row(&mut row, &mut field, &mut rows),
                _ => field.push(c),
            }
        }
        if rows.len() > MAX_ROWS {
            bail!("слишком много строк (максимум {MAX_ROWS})");
        }
    }
    if in_quotes {
        bail!("в файле незакрытая кавычка — файл повреждён");
    }
    if !field.is_empty() || !row.is_empty() {
        end_row(&mut row, &mut field, &mut rows);
    }
    Ok(rows)
}

fn quote(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn write(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for r in rows {
        let line: Vec<String> = r.iter().map(|f| quote(f)).collect();
        out.push_str(&line.join(","));
        out.push_str("\r\n");
    }
    out
}

pub const EXPORT_HEADER: [&str; 5] = ["name", "url", "username", "password", "note"];

fn find(header: &[String], names: &[&str]) -> Option<usize> {
    header
        .iter()
        .position(|h| names.contains(&h.trim().to_lowercase().as_str()))
}

pub struct Parsed {
    pub items: Vec<NewCredential>,
    /// Строки без пароля или без адреса/названия (например, заметки и карты из Bitwarden).
    pub skipped: usize,
}

pub fn credentials_from_csv(text: &str) -> Result<Parsed> {
    let rows = parse(text)?;
    let Some((header, body)) = rows.split_first() else {
        bail!("файл пустой");
    };
    let Some(pw) = find(header, &["password", "login_password", "pass", "pwd"]) else {
        bail!("не найден столбец с паролем (password). Это точно CSV с паролями?");
    };
    let url = find(header, &["url", "login_uri", "origin", "website", "web site", "uri", "login uri"]);
    let name = find(header, &["name", "title", "account"]);
    let user = find(
        header,
        &["username", "login_username", "login", "user", "user name", "user_name", "email"],
    );
    let note = find(header, &["note", "notes"]);
    if url.is_none() && name.is_none() {
        bail!("не найден столбец с адресом сайта (url) или названием (name)");
    }

    let get = |r: &Vec<String>, i: Option<usize>| -> String {
        i.and_then(|i| r.get(i)).cloned().unwrap_or_default()
    };
    let mut items = Vec::new();
    let mut skipped = 0;
    for r in body {
        let c = NewCredential {
            name: get(r, name),
            url: get(r, url),
            username: get(r, user),
            password: r.get(pw).cloned().unwrap_or_default(),
            note: get(r, note),
        };
        if c.password.is_empty() || (c.url.trim().is_empty() && c.name.trim().is_empty()) {
            skipped += 1;
        } else {
            items.push(c);
        }
    }
    Ok(Parsed { items, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quotes_commas_newlines_and_crlf() {
        let t = "\u{feff}a,b,c\r\n\"x,1\",\"he said \"\"hi\"\"\",\"line1\nline2\"\r\n\r\nlast,,";
        let r = parse(t).unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[1], vec!["x,1", "he said \"hi\"", "line1\nline2"]);
        assert_eq!(r[2], vec!["last", "", ""]);
    }

    #[test]
    fn unterminated_quote_is_an_error() {
        assert!(parse("a,b\n\"oops,1\n").is_err());
    }

    #[test]
    fn write_then_parse_is_lossless() {
        let rows = vec![
            vec!["n".into(), "p,q".into(), "a\"b".into(), "l1\r\nl2".into(), " sp ".into()],
            vec!["Ünï".into(), "".into(), "=1+1".into(), "ключ".into(), "".into()],
        ];
        assert_eq!(parse(&write(&rows)).unwrap(), rows);
    }

    #[test]
    fn imports_chrome_format() {
        let t = "name,url,username,password,note\nexample.com,https://example.com/login,me,p@ss,\n";
        let p = credentials_from_csv(t).unwrap();
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.items[0].url, "https://example.com/login");
        assert_eq!(p.items[0].password, "p@ss");
    }

    #[test]
    fn imports_bitwarden_and_skips_notes_without_password() {
        let t = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
                 ,0,login,Site,my note,,0,https://a.com,u1,pw1,\n\
                 ,0,note,Just a note,text,,0,,,,\n";
        let p = credentials_from_csv(t).unwrap();
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.skipped, 1);
        assert_eq!(p.items[0].username, "u1");
        assert_eq!(p.items[0].note, "my note");
    }

    #[test]
    fn rejects_files_without_password_column() {
        assert!(credentials_from_csv("name,url\nx,y\n").is_err());
        assert!(credentials_from_csv("").is_err());
        assert!(credentials_from_csv("password\nabc\n").is_err());
    }
}
