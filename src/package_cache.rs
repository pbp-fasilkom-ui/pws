//! Points `docker build` at the package caches beside PWS.
//!
//! PyPI and the Alpine CDN are both served by Fastly, and from the production
//! host that path often crawls at a few hundred KB/s. A single Django wheel
//! could then take most of a minute, and builds ran into the build timeout.
//! The caches download each package once and serve it locally afterwards.
//!
//! The caches are offered as build args. A Dockerfile only sees an arg it
//! declares with `ARG`, so the generated Dockerfile declares them, and a
//! project's own Dockerfile opts in the same way.

use std::{collections::HashSet, time::Duration};

use tokio::{net::TcpStream, time::timeout};
use url::Url;

use crate::configuration::BuilderSettings;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Build args for every configured cache that is accepting connections.
///
/// A cache that is down is left out rather than failing the build: the build
/// then downloads from upstream as it did before the caches existed.
pub async fn build_args(settings: &BuilderSettings) -> Vec<(&'static str, String)> {
    let mut args = Vec::new();

    if let Some(index) = configured(&settings.pipindex) {
        if let Some(host) = reachable_host(index).await {
            args.push(("PIP_INDEX_URL", index.to_string()));
            // The cache is plain HTTP on the host's bridge address.
            args.push(("PIP_TRUSTED_HOST", host.clone()));
            args.push(("UV_DEFAULT_INDEX", index.to_string()));
            args.push(("UV_INSECURE_HOST", host));
        }
    }

    if let Some(mirror) = configured(&settings.apkmirror) {
        if reachable_host(mirror).await.is_some() {
            args.push(("APK_MIRROR", mirror.trim_end_matches('/').to_string()));
        }
    }

    args
}

/// Keeps the args `dockerfile` declares and the project does not set itself.
///
/// Passing an arg the Dockerfile never declares makes `docker build` print a
/// warning into the student's build log, and a project that sets one of these
/// as its own variable has chosen its index deliberately.
pub fn select(
    args: Vec<(&'static str, String)>,
    dockerfile: &str,
    is_project_variable: impl Fn(&str) -> bool,
) -> Vec<(&'static str, String)> {
    let declared = declared_args(dockerfile);
    args.into_iter()
        .filter(|(name, _)| declared.contains(name) && !is_project_variable(name))
        .collect()
}

fn configured(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// The host of `url` if something accepts a TCP connection at it.
async fn reachable_host(url: &str) -> Option<String> {
    let parsed = match Url::parse(url) {
        Ok(parsed) => parsed,
        Err(err) => {
            tracing::warn!(%url, %err, "Ignoring unparseable package cache URL");
            return None;
        }
    };
    let host = parsed.host_str()?.to_string();
    let port = parsed.port_or_known_default()?;

    match timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), port))).await {
        Ok(Ok(_)) => Some(host),
        Ok(Err(err)) => {
            tracing::warn!(%url, %err, "Package cache unreachable; building without it");
            None
        }
        Err(_) => {
            tracing::warn!(%url, "Package cache timed out; building without it");
            None
        }
    }
}

/// Names declared by `ARG` instructions, including `ARG NAME=default` and
/// several names on one line.
fn declared_args(dockerfile: &str) -> HashSet<&str> {
    dockerfile
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let instruction = words.next()?;
            instruction.eq_ignore_ascii_case("ARG").then_some(words)
        })
        .flatten()
        .map(|word| word.split('=').next().unwrap_or(word))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_args() -> Vec<(&'static str, String)> {
        vec![
            ("PIP_INDEX_URL", "http://cache/simple/".to_string()),
            ("PIP_TRUSTED_HOST", "cache".to_string()),
            ("APK_MIRROR", "http://cache/alpine".to_string()),
        ]
    }

    fn names(args: &[(&'static str, String)]) -> Vec<&'static str> {
        args.iter().map(|(name, _)| *name).collect()
    }

    #[test]
    fn keeps_only_declared_args() {
        let dockerfile = "FROM python:3.13-alpine\narg PIP_INDEX_URL\nRUN pip install x\n";
        let selected = select(all_args(), dockerfile, |_| false);
        assert_eq!(names(&selected), ["PIP_INDEX_URL"]);
    }

    #[test]
    fn reads_defaults_and_several_names_per_line() {
        let dockerfile =
            "ARG PIP_INDEX_URL=https://pypi.org/simple PIP_TRUSTED_HOST\nARG APK_MIRROR=\n";
        let selected = select(all_args(), dockerfile, |_| false);
        assert_eq!(
            names(&selected),
            ["PIP_INDEX_URL", "PIP_TRUSTED_HOST", "APK_MIRROR"]
        );
    }

    #[test]
    fn project_variables_win() {
        let dockerfile = "ARG PIP_INDEX_URL\nARG PIP_TRUSTED_HOST\n";
        let selected = select(all_args(), dockerfile, |name| name == "PIP_INDEX_URL");
        assert_eq!(names(&selected), ["PIP_TRUSTED_HOST"]);
    }

    #[test]
    fn mentions_outside_arg_instructions_do_not_count() {
        let dockerfile = "# ARG PIP_INDEX_URL\nRUN echo PIP_INDEX_URL\nENV APK_MIRROR=x\n";
        assert!(select(all_args(), dockerfile, |_| false).is_empty());
    }

    #[tokio::test]
    async fn unset_or_blank_settings_add_nothing() {
        let settings = BuilderSettings {
            max: 1,
            timeout: 1,
            pipindex: None,
            apkmirror: Some("  ".to_string()),
        };
        assert!(build_args(&settings).await.is_empty());
    }

    #[tokio::test]
    async fn unreachable_cache_is_skipped() {
        // Port 9 on loopback: nothing listens there, so the connect is refused.
        let settings = BuilderSettings {
            max: 1,
            timeout: 1,
            pipindex: Some("http://127.0.0.1:9/simple/".to_string()),
            apkmirror: None,
        };
        assert!(build_args(&settings).await.is_empty());
    }

    #[tokio::test]
    async fn reachable_cache_adds_pip_and_uv_args() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let settings = BuilderSettings {
            max: 1,
            timeout: 1,
            pipindex: Some(format!("http://127.0.0.1:{port}/root/pypi/+simple/")),
            apkmirror: Some(format!("http://127.0.0.1:{port}/alpine/")),
        };
        let args = build_args(&settings).await;
        assert_eq!(
            names(&args),
            [
                "PIP_INDEX_URL",
                "PIP_TRUSTED_HOST",
                "UV_DEFAULT_INDEX",
                "UV_INSECURE_HOST",
                "APK_MIRROR"
            ]
        );
        assert_eq!(args[1].1, "127.0.0.1");
        assert_eq!(args[4].1, format!("http://127.0.0.1:{port}/alpine"));
    }
}
