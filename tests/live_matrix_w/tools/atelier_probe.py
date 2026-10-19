"""Atelier auth probe for the VM: reads %APPDATA%\\rism\\config.toml inside
python (both flat iris_* and [iris] section shapes), prints status only —
never the credentials."""
import base64
import os
import sys
import tomllib
import urllib.error
import urllib.request

p = os.path.join(os.environ["APPDATA"], "rism", "config.toml")
s = tomllib.load(open(p, "rb"))
iris = s.get("iris", s)
base = (iris.get("base_url") or iris.get("iris_base_url")
        or "http://localhost:52773").rstrip("/")
user = iris.get("username") or iris.get("iris_username") or "_SYSTEM"
pw = os.environ.get("RISM_IRIS_PASSWORD", "SYS")
req = urllib.request.Request(base + "/api/atelier/")
req.add_header("Authorization",
               "Basic " + base64.b64encode(f"{user}:{pw}".encode()).decode())
try:
    r = urllib.request.urlopen(req, timeout=10)
    print("ATELIER", r.status)
except urllib.error.HTTPError as e:
    print("ATELIER", e.code)
    sys.exit(1)
except Exception as e:  # noqa: BLE001
    print("ATELIER FAIL", type(e).__name__)
    sys.exit(1)
