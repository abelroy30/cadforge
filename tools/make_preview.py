#!/usr/bin/env python3
"""Freeze a running cadforge into one self-contained HTML file.

Why: the real app is a browser UI served by the Rust backend over HTTP. On a
machine with no browser and no port you can open, that interface is invisible.
This tool snapshots a live instance into a single .html you can open from disk -
same CSS, same app.js, same three.js - with the API responses baked in and
fetch/EventSource shimmed to serve them.

It is a photograph of the interface, not a replacement for it: buttons that
change things will not reach the backend.

Usage:
    tools/make_preview.py [--base http://127.0.0.1:8787] [--out preview.html]
"""

import argparse
import json
import os
import sys
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)


def get(base, path):
    with urllib.request.urlopen(base + path) as r:
        return json.loads(r.read())


def read(*parts):
    with open(os.path.join(ROOT, *parts), 'r', encoding='utf-8') as f:
        return f.read()


def body_of(index_html):
    start = index_html.index('<body>')
    end = index_html.index('</body>')
    return index_html[start + len('<body>'):end]


def strip_scripts(fragment):
    out = []
    for line in fragment.splitlines():
        if '<script' in line and 'src=' in line:
            continue
        out.append(line)
    return '\n'.join(out)


SHIM = r"""
window.EventSource = class {
  constructor() { this._h = {}; setTimeout(() => this._emit('open', {}), 0); }
  addEventListener(t, fn) { (this._h[t] = this._h[t] || []).push(fn); }
  _emit(t, e) { (this._h[t] || []).forEach((f) => f(e)); }
  close() {}
};

(function () {
  const S = window.__CADFORGE_SNAPSHOT__;
  const reply = (body) => Promise.resolve({
    ok: true, status: 200, statusText: 'OK', json: () => Promise.resolve(body),
  });
  window.fetch = function (path) {
    const p = String(path);
    if (p === '/api/state') return reply(S.state);
    if (p === '/api/trainer') return reply(S.trainer);
    let m = p.match(/\/api\/assets\/([^/]+)\/mesh$/);
    if (m && S.mesh[m[1]]) return reply(S.mesh[m[1]]);
    m = p.match(/\/api\/assets\/([^/]+)\/history$/);
    if (m && S.history[m[1]]) return reply(S.history[m[1]]);
    return Promise.resolve({
      ok: false, status: 404, statusText: 'not in snapshot',
      json: () => Promise.resolve({ error: 'not in this snapshot' }),
    });
  };
})();
"""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--base', default='http://127.0.0.1:8787')
    ap.add_argument('--out', default=os.path.join(ROOT, 'preview', 'cadforge-preview.html'))
    args = ap.parse_args()

    state = get(args.base, '/api/state')
    trainer = get(args.base, '/api/trainer')

    mesh, history = {}, {}
    for asset in state.get('assets', []):
        aid = asset['id']
        try:
            if asset['kind'] == 'mesh':
                mesh[aid] = get(args.base, '/api/assets/%s/mesh' % aid)
            elif asset['kind'] == 'step':
                history[aid] = get(args.base, '/api/assets/%s/history' % aid)
        except Exception as e:
            sys.stderr.write('skipping %s: %s\n' % (aid, e))

    snapshot = {'state': state, 'trainer': trainer, 'mesh': mesh, 'history': history}

    banner = (
        '<div id="preview-banner">Offline preview - this is the real interface and real '
        'part data captured from a running cadforge. Buttons that change things need the '
        'backend; run <code>cargo run</code> and open http://localhost:8787 for the live app.</div>'
    )
    banner_css = (
        '<style>#preview-banner{background:#1f6feb;color:#fff;padding:7px 12px;'
        'font:12px/1.4 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;'
        'flex:0 0 auto}#preview-banner code{font-family:ui-monospace,Menlo,Consolas,monospace;'
        'background:rgba(0,0,0,.22);padding:1px 4px;border-radius:3px}</style>'
    )

    html = '\n'.join([
        '<!doctype html>',
        '<html lang="en">',
        '<head>',
        '<meta charset="utf-8" />',
        '<meta name="viewport" content="width=device-width, initial-scale=1" />',
        '<title>cadforge - offline preview</title>',
        '<style>' + read('static', 'app.css') + '</style>',
        banner_css,
        '</head>',
        '<body>',
        banner,
        strip_scripts(body_of(read('static', 'index.html'))),
        '<script>window.__CADFORGE_SNAPSHOT__ = ' + json.dumps(snapshot) + ';</script>',
        '<script>' + read('static', 'vendor', 'three.min.js') + '</script>',
        '<script>' + read('static', 'vendor', 'OrbitControls.js') + '</script>',
        '<script>' + SHIM + '</script>',
        '<script>' + read('static', 'app.js') + '</script>',
        '</body>',
        '</html>',
        '',
    ])

    out_dir = os.path.dirname(args.out)
    if out_dir:
        os.makedirs(out_dir, exist_ok=True)
    with open(args.out, 'w', encoding='utf-8') as f:
        f.write(html)
    sys.stderr.write('wrote %s (%d KB) - %d assets, %d meshes, %d histories\n' % (
        args.out, len(html) // 1024, len(state.get('assets', [])), len(mesh), len(history)))
    return 0


if __name__ == '__main__':
    sys.exit(main())
