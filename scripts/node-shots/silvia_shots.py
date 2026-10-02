#!/usr/bin/env python3
"""Screenshot every silvia node (or the slugs given) from a headless Chromium.

Usage: silvia_shots.py OUT_DIR [slug ...]
Needs silvia served at $SILVIA_URL (default http://127.0.0.1:8080; `shots.sh silvia` serves
the checkout at $SILVIA, default ~/silvia, for you) and the python package playwright, with
playwright's chromium-headless-shell installed (`shots.sh setup`).
Writes OUT_DIR/<slug>.png (2x) and one JSON line per node -- ports, controls with defaults,
options, values, custom DOM, header buttons, text -- to OUT_DIR/index.jsonl.
"""
import sys, json, os, re, glob, asyncio
from playwright.async_api import async_playwright

SILVIA = os.environ.get("SILVIA", os.path.expanduser("~/silvia"))
URL = os.environ.get("SILVIA_URL", "http://127.0.0.1:8080")
OUT = None

def all_slugs():
    slugs = set()
    # silvia's video nodes were js/nodes/*.js and are now js/nodes/video/*.js; read both, and
    # leave js/nodes/audio/ out of the default list: this review compares the video graph, and
    # a node that moved there (audioanalyzer) is still reachable by naming its slug.
    files = glob.glob(os.path.join(SILVIA, "js/nodes/*.js")) + glob.glob(
        os.path.join(SILVIA, "js/nodes/video/*.js")
    )
    for f in files:
        for m in re.finditer(r"slug:\s*'([a-z0-9_]+)'", open(f).read()):
            slugs.add(m.group(1))
    slugs.discard("template")
    return sorted(slugs)

# Runs in the page: clear the workspace, make one node of this slug, wait for its custom UI,
# and describe it.
ADD = """
async (slug) => {
  const {SNode} = await import('/js/snode.js');
  try { (await import('/js/editor.js')).clearWorkspace(); } catch(e) {}
  document.querySelectorAll('.node, .node-root').forEach(n => n.remove());
  const n = new SNode(slug, 80, 80);
  await new Promise(r => setTimeout(r, 600));
  const el = n.nodeEl;
  const txt = e => (e.textContent||'').trim().replace(/\\s+/g,' ');
  return {
    slug, label: n.label, icon: n.icon, tooltip: n.tooltip,
    inputs: Object.entries(n.input).map(([k,p]) => ({key:k, label:p.label, type:p.type, control:p.control===null?null:(p.control||{}), unit:p.control?.unit, samplingCost:p.samplingCost||p.control?.samplingCost})),
    outputs: Object.entries(n.output).map(([k,p]) => ({key:k, label:p.label, type:p.type})),
    options: n.options ? Object.entries(n.options).map(([k,o]) => ({key:k, label:o.label, default:o.default, choices:o.choices?.map(c=>c.value??c)})) : null,
    values: n.values || null,
    customUI: el ? Array.from(el.querySelectorAll('.node-custom *')).filter(e=>e.children.length===0).map(e=>e.tagName.toLowerCase()+(e.className?'.'+String(e.className).split(' ').join('.'):'')+(txt(e)?'['+txt(e).slice(0,40)+']':'')).slice(0,60) : null,
    headerButtons: el ? Array.from(el.querySelectorAll('.node-header button, .node-header [title]')).map(e=>e.title||txt(e)) : null,
    domText: el ? txt(el).slice(0,600) : null,
    bbox: el ? el.getBoundingClientRect().toJSON() : null,
  };
}
"""

async def main(slugs):
    index = open(os.path.join(OUT, "index.jsonl"), "a")
    async with async_playwright() as p:
        b = await p.chromium.launch(args=["--use-gl=angle", "--use-angle=swiftshader",
                                          "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"])
        pg = await b.new_page(viewport={"width": 1600, "height": 1000}, device_scale_factor=2)
        errs = []; pg.on("pageerror", lambda e: errs.append(str(e)))
        # first visit: mark the About dialog as seen for this version, then load for real
        await pg.goto(URL + "/index.html", wait_until="load")
        await pg.wait_for_timeout(1500)
        await pg.evaluate("async () => { const v = await import('/js/version.js'); localStorage.setItem('silvia_last_seen_version', v.getCurrentVersion()); }")
        await pg.goto(URL + "/index.html", wait_until="load")
        await pg.wait_for_timeout(2500)
        await pg.evaluate("() => document.querySelectorAll('dialog[open], .modal.open, .modal-overlay').forEach(m => { try { m.close && m.close(); } catch(e){}; m.remove(); })")
        for slug in slugs:
            try:
                info = await pg.evaluate(ADD, slug)
                bb = info.get("bbox")
                path = os.path.join(OUT, f"{slug}.png")
                if bb and bb["width"] > 0:
                    await pg.screenshot(path=path, clip={"x": max(bb["x"] - 10, 0), "y": max(bb["y"] - 10, 0),
                                                         "width": bb["width"] + 20, "height": bb["height"] + 20})
                else:
                    await pg.screenshot(path=path)
                info["png"] = path
                print(slug, "ok", file=sys.stderr)
            except Exception as e:
                info = {"slug": slug, "error": str(e)}
                print(slug, "ERROR", e, file=sys.stderr)
            index.write(json.dumps(info, ensure_ascii=False) + "\n"); index.flush()
        if errs: print("PAGE ERRORS:", errs[:5], file=sys.stderr)
        await b.close()

if __name__ == "__main__":
    if len(sys.argv) < 2: print(__doc__); sys.exit(2)
    OUT = sys.argv[1]; os.makedirs(OUT, exist_ok=True)
    asyncio.run(main(sys.argv[2:] or all_slugs()))
