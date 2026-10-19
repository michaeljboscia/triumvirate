import json, pathlib, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
port, initial, state = int(sys.argv[1]), pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3])
state.write_text(initial.read_text())
class H(BaseHTTPRequestHandler):
 def log_message(self,*_): pass
 def send(self,n,v):
  b=json.dumps(v).encode(); self.send_response(n); self.send_header("Content-Type","application/json"); self.send_header("Content-Length",str(len(b))); self.end_headers(); self.wfile.write(b)
 def do_GET(self): self.send(200,json.loads(state.read_text()))
 def do_POST(self):
  req=json.loads(self.rfile.read(int(self.headers.get("Content-Length","0"))) or b"{}"); data=json.loads(state.read_text()); seat=req.get("agent",""); row=data.get("seats",{}).get(seat,{})
  code=data.get("_refusal_codes",{}).get(seat) or ("seat_not_configured" if not row.get("configured") else "seat_suspended" if row.get("suspended") else "seat_unverified" if not row.get("available") else "")
  self.send(502,{"error":f"{code}: {seat}: fixture"}) if code else self.send(200,{"response":"OK"})
HTTPServer(("127.0.0.1",port),H).serve_forever()
