#!/usr/bin/env python3
"""Serve a built site the way Cloudflare Pages serves it, for the local SSG checks.

The deployed site's routing lives in Pages, not in the app: `_redirects`, the HTML route matching
and the 404 page. The smoke, render and screenshot checks serve the build with this one server so a
local run answers each path with the status and the document production would.

For a request path, in order:

1. A `_redirects` rule with a 3xx status whose source is the path answers with that redirect.
   Pages applies these before it looks for a file, which is why no prerendered page may be a
   source (`redirects_lock`).
2. A file of the build answers, matched as Pages matches HTML: `/a` serves `a.html`, `/a/` serves
   `a/index.html`, and the other spelling of each answers 308 to it (`/a/` to `/a` when only
   `a.html` exists, `/a` to `/a/` when only `a/index.html` exists, `/a.html` to `/a` and
   `/a/index.html` to `/a/`). Any other file answers as itself.
3. A `_redirects` rule with status 200 (a rewrite) whose source matches serves its destination.
   A source ending in `/*` matches every path below it.
4. The nearest `404.html`, from the path's directory up to the root, answers with status 404.
5. With no `404.html` anywhere, Pages takes the site for a single-page app and serves
   `/index.html` with 200.

Usage: python3 scripts/web/pages-server.py <built-public-dir> <port>
       Port 0 picks a free port. The first line on stdout is `listening on <port>`.
"""

import functools
import http.server
import os
import posixpath
import sys
import urllib.parse


def read_rules(root):
    """The `_redirects` rules as (source, destination, status), comments and blank lines skipped."""
    path = os.path.join(root, "_redirects")
    if not os.path.isfile(path):
        return []
    rules = []
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            fields = line.split()
            if not fields or fields[0].startswith("#"):
                continue
            source, destination = fields[0], fields[1]
            status = int(fields[2]) if len(fields) > 2 else 302
            rules.append((source, destination, status))
    return rules


def rule_matches(source, path):
    if source.endswith("/*"):
        prefix = source[:-1]
        return path.startswith(prefix) or path == source[:-2]
    return path == source


class PagesHandler(http.server.SimpleHTTPRequestHandler):
    rules = []

    def file_of(self, url_path):
        return self.translate_path(url_path)

    def is_file(self, url_path):
        return os.path.isfile(self.file_of(url_path))

    def answer_redirect(self, status, location):
        self.send_response(status)
        self.send_header("Location", location)
        self.send_header("Content-Length", "0")
        self.end_headers()
        return None

    def answer_file(self, url_path, status=200):
        file = self.file_of(url_path)
        handle = open(file, "rb")
        size = os.fstat(handle.fileno()).st_size
        self.send_response(status)
        self.send_header("Content-Type", self.guess_type(file))
        self.send_header("Content-Length", str(size))
        self.end_headers()
        return handle

    def matched_asset(self, path):
        """What Pages answers from the build's files: ('file', url) or ('redirect', location), or None."""
        if path.endswith("/index.html"):
            if self.is_file(path):
                return ("redirect", path[: -len("index.html")])
            return None
        if path.endswith(".html") and self.is_file(path):
            return ("redirect", path[: -len(".html")] or "/")
        if path.endswith("/"):
            if self.is_file(path + "index.html"):
                return ("file", path + "index.html")
            if path != "/" and self.is_file(path[:-1] + ".html"):
                return ("redirect", path[:-1])
            return None
        if self.is_file(path):
            return ("file", path)
        if self.is_file(path + ".html"):
            return ("file", path + ".html")
        if self.is_file(path + "/index.html"):
            return ("redirect", path + "/")
        return None

    def nearest_not_found(self, path):
        directory = posixpath.dirname(path)
        while True:
            candidate = posixpath.join(directory, "404.html")
            if self.is_file(candidate):
                return candidate
            if directory in ("", "/"):
                return None
            directory = posixpath.dirname(directory)

    def send_head(self):
        path = urllib.parse.urlsplit(self.path).path or "/"
        query = urllib.parse.urlsplit(self.path).query
        for source, destination, status in self.rules:
            if 300 <= status < 400 and rule_matches(source, path):
                return self.answer_redirect(status, destination)
        asset = self.matched_asset(path)
        if asset is not None:
            kind, target = asset
            if kind == "redirect":
                return self.answer_redirect(308, target + (f"?{query}" if query else ""))
            return self.answer_file(target)
        for source, destination, status in self.rules:
            if status == 200 and rule_matches(source, path) and self.is_file(destination):
                return self.answer_file(destination)
        not_found = self.nearest_not_found(path)
        if not_found is not None:
            return self.answer_file(not_found, status=404)
        return self.answer_file("/index.html")

    def log_message(self, *args):
        pass


def main():
    root, port = sys.argv[1], int(sys.argv[2])
    handler = functools.partial(PagesHandler, directory=root)
    PagesHandler.rules = read_rules(root)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), handler)
    print(f"listening on {server.server_address[1]}", flush=True)
    server.serve_forever()


main()
