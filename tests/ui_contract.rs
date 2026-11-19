//! The interface contract, checked mechanically.
//!
//! A missing element id is the one bug a browser reports only at the moment a
//! user clicks: `$("#thing")` returns null and the handler throws. There is no
//! browser in this build, so the contract is checked here instead - every id the
//! script reaches for must exist in the page, and every API path it calls must be
//! a route the server actually registers.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

/// Every `id="..."` in the page.
fn ids_in_html(html: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes = html.as_bytes();
    let mut i = 0;
    while let Some(rel) = html[i..].find("id=\"") {
        let start = i + rel + 4;
        let end = start + html[start..].find('"').expect("unterminated id attribute");
        out.insert(html[start..end].to_string());
        i = end;
        // Keep the borrow checker and the loop honest about progress.
        if i >= bytes.len() {
            break;
        }
    }
    out
}

/// Every id the script reaches for, by either `$("#id")` or
/// `getElementById("id")`.
fn ids_used_by_js(js: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (marker, skip) in [("$(\"#", 4usize), ("getElementById(\"", 16usize)] {
        let mut i = 0;
        while let Some(rel) = js[i..].find(marker) {
            let start = i + rel + skip;
            if let Some(len) = js[start..].find('"') {
                out.insert(js[start..start + len].to_string());
                i = start + len;
            } else {
                break;
            }
        }
    }
    out
}

/// Every `"/api..."` string literal in the script.
fn api_literals(js: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = js[i..].find("\"/api") {
        let start = i + rel + 1;
        if let Some(len) = js[start..].find('"') {
            out.push(js[start..start + len].to_string());
        }
        i = start + 1;
    }
    out
}

/// Every route the server registers.
fn routes() -> Vec<String> {
    let api = read("src/api.rs");
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = api[i..].find(".route(\"") {
        let start = i + rel + 8;
        if let Some(len) = api[start..].find('"') {
            out.push(api[start..start + len].to_string());
        }
        i = start + 1;
    }
    out
}

#[test]
fn every_id_the_script_uses_exists_in_the_page() {
    let defined = ids_in_html(&read("static/index.html"));
    let used = ids_used_by_js(&read("static/app.js"));
    assert!(!used.is_empty(), "the script references no elements at all");
    let missing: Vec<&String> = used.difference(&defined).collect();
    assert!(
        missing.is_empty(),
        "these ids are used by static/app.js but not defined in static/index.html: {missing:?}"
    );
}

#[test]
fn every_api_path_the_script_calls_is_a_real_route() {
    let routes = routes();
    assert!(routes.len() > 10, "only found {} routes", routes.len());

    for literal in api_literals(&read("static/app.js")) {
        // A query string is not part of the path being routed.
        let path: String = literal.split('?').next().unwrap_or(&literal).to_string();
        let trimmed = path.trim_end_matches('/');
        let known = routes
            .iter()
            .any(|r| r == &path || r.starts_with(&format!("{trimmed}/")) || r == trimmed);
        assert!(
            known,
            "static/app.js calls {path:?}, which matches no route in src/api.rs; 
             routes are {routes:?}"
        );
    }
}

#[test]
fn path_fragments_the_script_builds_are_not_typos() {
    let routes = routes();
    let js = read("static/app.js");
    // Fragments are the `/mesh`, `/history`, `/preview` halves of a concatenated
    // URL. Each one has to appear in some registered route.
    for fragment in [
        "/mesh", "/history", "/preview", "/streams", "/file", "/cancel",
    ] {
        if js.contains(&format!("\"{fragment}\"")) {
            assert!(
                routes.iter().any(|r| r.contains(fragment)),
                "static/app.js builds {fragment:?} but no route contains it"
            );
        }
    }
}

#[test]
fn the_page_is_completely_offline() {
    for file in ["static/index.html", "static/app.js", "static/app.css"] {
        let text = read(file);
        for needle in ["http://", "https://", "//cdn", "fonts.googleapis"] {
            // A comment is allowed to mention a URL; code is not.
            for (n, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                let commented = trimmed.starts_with("//")
                    || trimmed.starts_with('*')
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('#');
                assert!(
                    commented || !line.contains(needle),
                    "{file}:{} reaches the network: {needle}",
                    n + 1
                );
            }
        }
    }
}

#[test]
fn the_page_loads_its_scripts_in_order() {
    let html = read("static/index.html");
    let three = html
        .find("/vendor/three.min.js")
        .expect("three.js is not loaded");
    let controls = html
        .find("/vendor/OrbitControls.js")
        .expect("OrbitControls is not loaded");
    let app = html.find("/app.js").expect("app.js is not loaded");
    assert!(three < controls, "three.js must load before OrbitControls");
    assert!(
        controls < app,
        "the vendored scripts must load before app.js"
    );
}

/// Every value of one `data-<attr>="..."` on the page, in order.
fn data_attrs(html: &str, attr: &str) -> Vec<String> {
    let needle = format!("data-{attr}=\"");
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = html[i..].find(&needle) {
        let start = i + rel + needle.len();
        match html[start..].find('"') {
            Some(len) => {
                out.push(html[start..start + len].to_string());
                i = start + len;
            }
            None => break,
        }
    }
    out
}

/// A lane with no chip is an artifact that exists in the data and cannot be found
/// in the list: the filters would hide it and nothing would look wrong. The other
/// direction is the same bug - the studio must not offer a chip for a lane it
/// never produces.
#[test]
fn every_lane_the_studio_uses_can_be_filtered_to() {
    let html = read("static/index.html");
    let filters = data_attrs(&html, "filter");
    let model = read("src/model.rs");

    // Lanes the server assigns by file type.
    for lane in ["sldworks", "step", "image", "other"] {
        assert!(
            filters.iter().any(|f| f == lane),
            "no filter chip for the {lane:?} lane, so those artifacts cannot be found: {filters:?}"
        );
        assert!(
            model.contains(&format!("\"{lane}\"")),
            "the page offers a {lane:?} chip that src/model.rs never assigns"
        );
    }

    // Lanes the studio creates itself when it captures views and sheets.
    for lane in ["view", "drawing"] {
        assert!(
            filters.iter().any(|f| f == lane),
            "no filter chip for the {lane:?} lane: {filters:?}"
        );
    }
}
/// A duplicate id is a quiet bug of a particular kind: the page still renders,
/// but `$("#x")` binds whichever of the two came first, so a control silently
/// stops working while a different one moves. Worth a test of its own.
#[test]
fn every_element_id_is_unique() {
    let html = read("static/index.html");
    let mut seen: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(rel) = html[i..].find("id=\"") {
        let start = i + rel + 4;
        match html[start..].find('"') {
            Some(len) => {
                seen.push(html[start..start + len].to_string());
                i = start + len;
            }
            None => break,
        }
    }
    assert!(seen.len() > 30, "only found {} ids on the page", seen.len());

    let mut dupes: Vec<&String> = Vec::new();
    for id in &seen {
        if seen.iter().filter(|s| *s == id).count() > 1 && !dupes.contains(&id) {
            dupes.push(id);
        }
    }
    assert!(
        dupes.is_empty(),
        "these ids appear more than once in static/index.html: {dupes:?}"
    );
}

/// The three drop targets are the promise the studio makes: CAD files, STEP
/// files and pictures, each on its own lane. A typo in a lane name would make an
/// upload land on a lane nothing filters by, so the names are checked here
/// against the ones the studio actually accepts.
#[test]
fn the_upload_tiles_name_lanes_the_studio_accepts() {
    let html = read("static/index.html");
    let known = ["sldworks", "step", "image", "view", "drawing", "other"];

    let lanes = data_attrs(&html, "lane");
    assert!(!lanes.is_empty(), "the page offers no upload lane");
    for lane in &lanes {
        assert!(
            known.contains(&lane.as_str()),
            "static/index.html uses lane {lane:?}, which is not one the studio accepts: {known:?}"
        );
    }

    for (id, lane) in [
        ("lane-sldworks", "sldworks"),
        ("lane-step", "step"),
        ("lane-image", "image"),
    ] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "the page has no {id} tile"
        );
        assert!(
            html.contains(&format!("data-lane=\"{lane}\"")),
            "no upload tile is wired to the {lane} lane"
        );
    }
}

#[test]
fn the_artifact_filters_name_real_lanes() {
    let html = read("static/index.html");
    let known = [
        "all", "sldworks", "step", "image", "view", "drawing", "other",
    ];
    let filters = data_attrs(&html, "filter");
    assert!(!filters.is_empty(), "the artifact list has no filter chips");
    for filter in &filters {
        assert!(
            known.contains(&filter.as_str()),
            "static/index.html filters by {filter:?}, which is not a lane: {known:?}"
        );
    }
}
