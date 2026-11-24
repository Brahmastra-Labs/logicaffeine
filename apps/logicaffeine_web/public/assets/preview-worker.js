// The Studio's page builder: compiles a Web page's preview beside the page, so an edit never holds
// the Studio still (apps/logicaffeine_web/src/ui/pages/preview_builds.rs).
//
// It loads the site's own bundle, whose entry returns at once where there is no window, and calls
// the one function the bundle exports for it. The page sends one source at a time.

let bundle = null;

const load = (entry) => {
  bundle ??= import(entry).then(async (glue) => {
    while (!globalThis.__dx_mainWasm) await new Promise((resolve) => setTimeout(resolve, 10));
    return glue;
  });
  return bundle;
};

self.onmessage = async ({ data: { entry, source } }) => {
  try {
    const glue = await load(entry);
    self.postMessage({ doc: glue.preview_document_in_worker(source) });
  } catch (error) {
    self.postMessage({ error: String(error) });
  }
};
