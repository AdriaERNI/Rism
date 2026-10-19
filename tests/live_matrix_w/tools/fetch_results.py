"""Zip the current results dir and print it as wrapped base64 (WinRM-safe:
the cmd layer preserves newlines; we avoid Out-String's console wrapping)."""
import base64
import glob
import io
import json
import zipfile

z = io.BytesIO()
zf = zipfile.ZipFile(z, "w")
rows = {}
for p in glob.glob(r"C:\RismL\live_matrix_w\results\*.json"):
    name = p.split("\\")[-1]
    zf.writestr(name, open(p, "rb").read())
    d = json.loads(open(p, encoding="utf-8").read())
    rows[d["id"]] = d["status"]
zf.close()
print("STATUS " + json.dumps(rows))
b = base64.b64encode(z.getvalue()).decode()
for i in range(0, len(b), 76):
    print(b[i:i + 76])
