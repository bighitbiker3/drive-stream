#!/usr/bin/env python3 -I
"""A fake Google Drive v3 API for exercising Drive Stream end to end.

Serves: the token endpoint, files.get / files.list, changes.*, and ranged
alt=media downloads with simulated latency and per-connection bandwidth.
File content is deterministic, so readers can verify any byte range.

POST /_mutate {"op": "rename"|"add"|"trash", ...} changes the drive and
appends to the change feed, like an edit made on another device.
"""
import hashlib
import json
import os
import random
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

LATENCY = float(sys.argv[2]) if len(sys.argv) > 2 else 0.15      # seconds to first byte
BANDWIDTH = float(sys.argv[3]) if len(sys.argv) > 3 else 4e6      # bytes/s per connection
UPLOAD_BPS = float(os.environ.get("MOCK_UPLOAD_BPS", "0"))
PATTERN_LEN = 1_000_003
PATTERN = random.Random(42).randbytes(PATTERN_LEN)
lock = threading.Lock()
stats = {"media_requests": 0, "media_bytes": 0, "list_requests": 0}


def content(file_id, start, end):
    """Bytes [start, end) of a file: a prime-length pattern rotated per file."""
    rot = int(hashlib.md5(file_id.encode()).hexdigest(), 16) % PATTERN_LEN
    out = bytearray()
    pos = start
    while pos < end:
        i = (pos + rot) % PATTERN_LEN
        take = min(end - pos, PATTERN_LEN - i)
        out += PATTERN[i:i + take]
        pos += take
    return bytes(out)


def f(id, name, parent, mime="video/quicktime", size=None):
    d = {"id": id, "name": name, "mimeType": mime, "parents": [parent],
         "modifiedTime": "2026-10-01T12:00:00.000Z", "trashed": False}
    if size is not None:
        d["size"] = str(size)
        d["md5Checksum"] = hashlib.md5(id.encode()).hexdigest()
    return d


FOLDER = "application/vnd.google-apps.folder"
files = {x["id"]: x for x in [
    f("ROOT", "My Drive", "", FOLDER),
    f("d_drone", "Videos", "ROOT", FOLDER),
    f("d_slash", "Before/After", "d_drone", FOLDER),
    f("v_big", "clip_0066.mov", "d_drone", size=2_076_247_140),
    f("v_med", "clip_0089.mov", "d_drone", size=300_000_000),
    f("v_slash", "clip.mp4", "d_slash", "video/mp4", size=50_000_000),
    f("dup1", "notes.txt", "ROOT", "text/plain", size=1000),
    f("dup2", "notes.txt", "ROOT", "text/plain", size=2000),
    f("small", "hello.txt", "ROOT", "text/plain", size=17),
    f("doc1", "Budget 2026", "ROOT", "application/vnd.google-apps.spreadsheet"),
]}
files["ROOT"]["parents"] = []
changes = []  # change feed entries
uploaded = {}  # file id -> bytes, for files written through the API
sessions = {}  # upload session id -> {"meta":..., "existing":..., "data": bytearray, "total": n}


def changed(fid):
    changes.append({"fileId": fid, "removed": False, "file": dict(files[fid])})


def finish_upload(sess):
    data = bytes(sess["data"])
    fid = sess["existing"] or "up_" + hashlib.md5(os.urandom(8)).hexdigest()[:10]
    if fid not in files:
        m = sess["meta"]
        files[fid] = f(fid, m["name"], m["parents"][0], m.get("mimeType", "application/octet-stream"), len(data))
    files[fid]["size"] = str(len(data))
    files[fid]["md5Checksum"] = hashlib.md5(data).hexdigest()
    files[fid]["modifiedTime"] = "2026-11-02T13:00:00.000Z"
    uploaded[fid] = data
    stats["uploads"] = stats.get("uploads", 0) + 1
    changed(fid)
    return files[fid]


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def send_json(self, obj, code=200):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        u = urlparse(self.path)
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n)
        if u.path == "/token":
            form = {k: v[0] for k, v in parse_qs(body.decode()).items()}
            out = {"access_token": "fake-token", "expires_in": 3600, "scope": "https://www.googleapis.com/auth/drive"}
            if form.get("grant_type") == "authorization_code":
                if form.get("code") != "good-code":
                    return self.send_json({"error": "invalid_grant"}, 400)
                out["refresh_token"] = "fake-refresh"
            return self.send_json(out)
        if u.path == "/drive/v3/files":
            m = json.loads(body)
            with lock:
                fid = "fold_" + hashlib.md5(os.urandom(8)).hexdigest()[:10]
                files[fid] = f(fid, m["name"], m["parents"][0], m["mimeType"])
                changed(fid)
                return self.send_json(files[fid])
        if u.path.startswith("/upload/drive/v3/files"):
            return self.start_session(None, body)
        if u.path == "/_mutate":
            m = json.loads(body)
            with lock:
                if m["op"] == "rename":
                    files[m["id"]]["name"] = m["name"]
                    files[m["id"]]["modifiedTime"] = "2026-11-02T12:00:00.000Z"
                    changes.append({"fileId": m["id"], "removed": False, "file": dict(files[m["id"]])})
                elif m["op"] == "add":
                    files[m["id"]] = f(m["id"], m["name"], m["parent"], m.get("mime", "text/plain"), m.get("size"))
                    changes.append({"fileId": m["id"], "removed": False, "file": dict(files[m["id"]])})
                elif m["op"] == "fail_uploads":
                    stats["fail_uploads"] = bool(m["on"])
                elif m["op"] == "trash":
                    files[m["id"]]["trashed"] = True
                    changes.append({"fileId": m["id"], "removed": False, "file": dict(files[m["id"]])})
                    del files[m["id"]]
            return self.send_json({"ok": True, "seq": len(changes)})
        self.send_json({"error": "nope"}, 404)

    def start_session(self, existing, body):
        if stats.get("fail_uploads"):
            return self.send_json({"error": {"code": 403, "message": "storageQuotaExceeded"}}, 403)
        sid = hashlib.md5(os.urandom(8)).hexdigest()
        with lock:
            sessions[sid] = {"meta": json.loads(body or b"{}"), "existing": existing, "data": bytearray(),
                             "total": int(self.headers.get("X-Upload-Content-Length", 0))}
        self.send_response(200)
        self.send_header("Location", f"http://127.0.0.1:{self.server.server_address[1]}/upload/session/{sid}")
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_PATCH(self):
        u = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(u.query).items()}
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0))
        fid = u.path.rsplit("/", 1)[1]
        if u.path.startswith("/upload/drive/v3/files/"):
            return self.start_session(fid, body)
        m = json.loads(body or b"{}")
        with lock:
            if fid not in files:
                return self.send_json({"error": {"code": 404}}, 404)
            if m.get("trashed"):
                files[fid]["trashed"] = True
                changed(fid)
                out = files.pop(fid)
                return self.send_json(out)
            if "name" in m:
                files[fid]["name"] = m["name"]
            if "addParents" in q:
                files[fid]["parents"] = [q["addParents"]]
            changed(fid)
            return self.send_json(files[fid])

    def do_PUT(self):
        u = urlparse(self.path)
        sid = u.path.rsplit("/", 1)[1]
        body = self.rfile.read(int(self.headers.get("Content-Length") or 0))
        if UPLOAD_BPS:
            time.sleep(len(body) / UPLOAD_BPS)  # imitate a home uplink
        cr = self.headers.get("Content-Range", "")
        with lock:
            sess = sessions[sid]
            if cr.startswith("bytes */"):
                if len(sess["data"]) >= sess["total"]:
                    return self.send_json(finish_upload(sess))
            else:
                a = int(cr.split(" ")[1].split("-")[0])
                if a == len(sess["data"]):
                    sess["data"] += body
            if len(sess["data"]) >= sess["total"]:
                return self.send_json(finish_upload(sess))
            self.send_response(308)
            self.send_header("Range", f"bytes=0-{len(sess['data']) - 1}")
            self.send_header("Content-Length", "0")
            self.end_headers()

    def do_GET(self):
        u = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(u.query).items()}
        if u.path == "/_stats":
            return self.send_json(stats)
        if u.path == "/drive/v3/about":
            return self.send_json({"user": {"emailAddress": os.environ.get("MOCK_EMAIL", "mock@example.com")}})
        if u.path == "/drive/v3/changes/startPageToken":
            return self.send_json({"startPageToken": str(len(changes))})
        if u.path == "/drive/v3/changes":
            start = int(q["pageToken"])
            with lock:
                batch = changes[start:]
                return self.send_json({"newStartPageToken": str(len(changes)), "changes": batch})
        if u.path == "/drive/v3/files":
            with lock:
                stats["list_requests"] += 1
                return self.send_json({"files": [x for x in files.values() if x["id"] != "ROOT"]})
        if u.path.startswith("/drive/v3/files/"):
            fid = u.path.rsplit("/", 1)[1]
            fid = "ROOT" if fid == "root" else fid
            if fid not in files:
                return self.send_json({"error": {"code": 404}}, 404)
            if q.get("alt") != "media":
                return self.send_json(files[fid])
            return self.media(files[fid])
        self.send_json({"error": "nope"}, 404)

    def media(self, meta):
        size = int(meta.get("size", 0))
        rng = self.headers.get("Range", "")
        start, end = 0, size
        if rng.startswith("bytes="):
            a, b = rng[6:].split("-")
            start, end = int(a), min(int(b) + 1 if b else size, size)
        time.sleep(LATENCY)
        data = uploaded[meta["id"]][start:end] if meta["id"] in uploaded else content(meta["id"], start, end)
        with lock:
            stats["media_requests"] += 1
            stats["media_bytes"] += len(data)
        self.send_response(206 if rng else 200)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Content-Range", f"bytes {start}-{end - 1}/{size}")
        self.end_headers()
        # Trickle at BANDWIDTH to imitate one Drive connection.
        chunk = 256 * 1024
        t0 = time.time()
        for i in range(0, len(data), chunk):
            self.wfile.write(data[i:i + chunk])
            ahead = (i + chunk) / BANDWIDTH - (time.time() - t0)
            if ahead > 0:
                time.sleep(ahead)


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18080
    print(f"mock drive on 127.0.0.1:{port} latency={LATENCY}s bandwidth={BANDWIDTH/1e6:.1f}MB/s per connection", flush=True)
    ThreadingHTTPServer(("127.0.0.1", port), H).serve_forever()
