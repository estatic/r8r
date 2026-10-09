//! `URL` and `URLSearchParams` for expressions and the Code node, as browsers
//! and Node.js have them (QuickJS has neither). Parsing is the `url` crate's
//! WHATWG parser; the JavaScript side is in `url_prelude.js`.

/// The JavaScript side: the classes over the `__r8r_url` natives.
const PRELUDE: &str = include_str!("url_prelude.js");

/// Longest URL the natives take: they allocate outside QuickJS's memory cap.
const MAX_URL: usize = 1_000_000;

/// Whether `script` may use `URL` or `URLSearchParams`.
pub fn is_used_by(script: &str) -> bool {
    script.contains("URL")
}

/// Installs `URL` and `URLSearchParams` into a fresh QuickJS context.
pub fn install(js: &rquickjs::Ctx<'_>) -> rquickjs::Result<()> {
    let natives = rquickjs::Object::new(js.clone())?;
    natives.set("parse", rquickjs::Function::new(js.clone(), parse_js)?)?;
    natives.set("set", rquickjs::Function::new(js.clone(), set_js)?)?;
    js.globals().set("__r8r_url", natives)?;
    js.eval::<(), _>(PRELUDE)
}

/// The URL's parts as JSON, or "" when it isn't a URL (the caller throws).
fn parse_js(input: String, base: rquickjs::function::Opt<String>) -> String {
    if input.len() > MAX_URL || base.0.as_ref().is_some_and(|b| b.len() > MAX_URL) {
        return String::new();
    }
    let parsed = match base.0 {
        Some(b) => url::Url::parse(&b).and_then(|b| b.join(&input)),
        None => url::Url::parse(&input),
    };
    parsed.map(|u| parts(&u)).unwrap_or_default()
}

/// `href` with one part (`protocol`, `hostname`, `pathname`, ...) set; the
/// new parts as JSON, or the old ones when the value doesn't fit.
fn set_js(href: String, part: String, value: String) -> String {
    let Ok(mut u) = url::Url::parse(&href) else {
        return String::new();
    };
    if value.len() > MAX_URL {
        return parts(&u);
    }
    // Like browsers, a value that doesn't fit leaves the URL as it was.
    let _ = match part.as_str() {
        "protocol" => u.set_scheme(value.trim_end_matches(':')),
        "username" => u.set_username(&value),
        "password" => u.set_password(Some(&value).filter(|v| !v.is_empty()).map(|v| v.as_str())),
        "host" => {
            let (host, port) = match value.rsplit_once(':') {
                Some((h, p))
                    if p.chars().all(|c| c.is_ascii_digit()) && !h.ends_with(']')
                        || h.ends_with(']') && !p.is_empty() =>
                {
                    (h, Some(p))
                }
                _ => (value.as_str(), None),
            };
            let ok = u.set_host(Some(host)).is_ok();
            if ok {
                if let Some(p) = port {
                    let _ = u.set_port(p.parse().ok());
                }
            }
            Ok(())
        }
        "hostname" => u.set_host(Some(&value)).map_err(|_| ()),
        "port" => u.set_port(if value.is_empty() {
            None
        } else {
            value.parse().ok()
        }),
        "pathname" => {
            u.set_path(&value);
            Ok(())
        }
        "search" => {
            let q = value.trim_start_matches('?');
            u.set_query(if q.is_empty() { None } else { Some(q) });
            Ok(())
        }
        "hash" => {
            let f = value.trim_start_matches('#');
            u.set_fragment(if f.is_empty() { None } else { Some(f) });
            Ok(())
        }
        _ => Ok(()),
    };
    parts(&u)
}

fn parts(u: &url::Url) -> String {
    let host = u.host_str().unwrap_or("");
    let port = u.port().map(|p| p.to_string()).unwrap_or_default();
    serde_json::json!({
        "href": u.as_str(),
        "protocol": format!("{}:", u.scheme()),
        "username": u.username(),
        "password": u.password().unwrap_or(""),
        "host": if port.is_empty() { host.to_string() } else { format!("{host}:{port}") },
        "hostname": host,
        "port": port,
        "pathname": u.path(),
        "search": u.query().filter(|q| !q.is_empty()).map(|q| format!("?{q}")).unwrap_or_default(),
        "hash": u.fragment().filter(|f| !f.is_empty()).map(|f| format!("#{f}")).unwrap_or_default(),
        "origin": u.origin().ascii_serialization(),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use crate::expr::{eval_js, EvalContext};
    use serde_json::json;

    fn run(script: &str) -> serde_json::Value {
        let items = vec![];
        let nodes = std::collections::HashMap::new();
        let ctx = EvalContext {
            json: json!({}),
            items: &items,
            node_json: &nodes,
            workflow_name: "",
            args: None,
        };
        eval_js(script, &ctx).unwrap()
    }

    #[test]
    fn parses_a_url_into_its_parts() {
        let parts = run("const u = new URL('https://user:pw@www.Example.com:8080/a/b.jpg?w=800&x=1#top'); \
            [u.protocol, u.username, u.password, u.host, u.hostname, u.port, u.pathname, u.search, u.hash, u.origin, String(u)]");
        assert_eq!(
            parts,
            json!([
                "https:",
                "user",
                "pw",
                "www.example.com:8080",
                "www.example.com",
                "8080",
                "/a/b.jpg",
                "?w=800&x=1",
                "#top",
                "https://www.example.com:8080",
                "https://user:pw@www.example.com:8080/a/b.jpg?w=800&x=1#top"
            ])
        );
    }

    #[test]
    fn resolves_against_a_base_and_throws_for_a_non_url() {
        assert_eq!(
            run("new URL('../c.png', 'https://x.org/a/b/').href"),
            json!("https://x.org/a/c.png")
        );
        assert_eq!(run("try { new URL('not a url'); 'no' } catch (e) { e instanceof TypeError && e.message.startsWith('Invalid URL') }"), json!(true));
        assert_eq!(
            run("[URL.canParse('https://a.b'), URL.canParse('nope'), URL.parse('nope')]"),
            json!([true, false, null])
        );
    }

    #[test]
    fn search_params_read_and_write_through_to_the_url() {
        let out = run("const u = new URL('https://x.org/s?q=cats+dogs&n=1&n=2'); \
            const p = u.searchParams; const got = [p.get('q'), p.getAll('n'), p.has('z')]; \
            p.set('n', '3'); p.append('lang', 'en gb'); p.delete('q'); \
            [got, u.href, u.search]");
        assert_eq!(
            out,
            json!([
                ["cats dogs", ["1", "2"], false],
                "https://x.org/s?n=3&lang=en+gb",
                "?n=3&lang=en+gb"
            ])
        );
    }

    #[test]
    fn url_search_params_on_their_own() {
        assert_eq!(
            run("new URLSearchParams({a: '1', b: 'x y'}).toString()"),
            json!("a=1&b=x+y")
        );
        assert_eq!(
            run("[...new URLSearchParams('?a=1&b=2').keys()]"),
            json!(["a", "b"])
        );
        assert_eq!(
            run("const s = new URLSearchParams([['z','1'],['a','2']]); s.sort(); s.toString()"),
            json!("a=2&z=1")
        );
    }

    #[test]
    fn setters_change_the_url() {
        assert_eq!(run("const u = new URL('http://a.com/x'); u.hostname = 'b.org'; u.pathname = '/y'; u.hash = 'h'; u.href"), json!("http://b.org/y#h"));
        assert_eq!(
            run("const u = new URL('http://a.com/x'); u.port = '81'; u.search = 'k=v'; u.href"),
            json!("http://a.com:81/x?k=v")
        );
    }
}
