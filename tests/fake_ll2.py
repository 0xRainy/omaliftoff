"""Fake LL2 (2.3.0 subset) + mock ntfy for tests/e2e.sh. Never talks to the network.

usage: fake_ll2.py <dir>   Binds 127.0.0.1:0 and writes the port to <dir>/port once listening.
Control files in <dir> (read on every request):
  net.txt     ISO NET of the one launch (id "e2e-1", a Falcon 9 from tests/fixtures/upcoming.json)
  prec.txt    LL2 net_precision abbrev (default MIN)
  status.txt  LL2 status id (default 1 = Go)
  fail.txt    if present: /launches/* answer 503
Logs: requests.log (GETs), requests.tlog (real epoch + GET), ntfy.log (every POST: path, Authorization, JSON body).
"""
import copy, http.server, json, os, sys, time

D = sys.argv[1]
HERE = os.path.dirname(os.path.abspath(__file__))
BASE = json.load(open(os.path.join(HERE, "fixtures", "upcoming.json")))["results"][0]
STATUS = {1: ("Go for Launch", "Go"), 3: ("Launch Successful", "Success"), 2: ("To Be Determined", "TBD"), 5: ("On Hold", "Hold"), 8: ("To Be Confirmed", "TBC")}


def rd(name, default=""):
    try:
        return open(os.path.join(D, name)).read().strip() or default
    except OSError:
        return default


def launch():
    l = copy.deepcopy(BASE)
    l["id"] = "e2e-1"
    l["net"] = l["window_start"] = l["window_end"] = rd("net.txt")
    p = rd("prec.txt", "MIN")
    l["net_precision"] = {"id": 0, "name": p, "abbrev": p, "description": ""}
    sid = int(rd("status.txt", "1"))
    name, abbrev = STATUS.get(sid, ("Status", "?"))
    l["status"] = {"id": sid, "name": name, "abbrev": abbrev, "description": ""}
    return l


class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def send(self, code, obj):
        b = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        with open(os.path.join(D, "requests.log"), "a") as f:
            f.write(self.path + "\n")
        with open(os.path.join(D, "requests.tlog"), "a") as f:
            f.write("%.3f %s\n" % (time.time(), self.path))
        if self.path.startswith("/api-throttle"):
            return self.send(200, {"your_request_limit": 15, "limit_frequency_secs": 3600, "current_use": 0, "next_use_secs": 0, "ident": "e2e"})
        if self.path.startswith("/launches") and os.path.exists(os.path.join(D, "fail.txt")):
            return self.send(503, {"detail": "fake outage"})
        if self.path.startswith("/launches/upcoming"):
            return self.send(200, {"count": 1, "next": None, "previous": None, "results": [launch()]})
        if self.path.startswith("/launches/"):
            return self.send(200, launch())
        self.send(404, {})

    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(n)
        with open(os.path.join(D, "ntfy.log"), "ab") as f:
            f.write(b"POST " + self.path.encode() + b" auth=" + str(self.headers.get("Authorization")).encode() + b" " + body + b"\n")
        self.send(200, {})


srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
with open(os.path.join(D, "port.tmp"), "w") as f:
    f.write(str(srv.server_address[1]))
os.rename(os.path.join(D, "port.tmp"), os.path.join(D, "port"))
srv.serve_forever()
