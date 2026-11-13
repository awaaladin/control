#!/data/data/com.termux/files/usr/bin/sh
# Use PULSE from anywhere through a Cloudflare quick tunnel, from the phone hub.
# Run in Termux next to start.sh:   ~/rhcs/remote.sh      (Ctrl+C stops sharing)
# Needs:  pkg install cloudflared     and rhcsd running (start.sh).
# Does not touch start.sh/stop.sh. The address changes every run.

dir="$(cd "$(dirname "$0")" && pwd)"
envf="$dir/.env"
pulse="https://rhcs-pulse.vercel.app"
port=7878

token=""
[ -f "$envf" ] && token="$(sed -n 's/^[[:space:]]*RHCSD_TOKEN[[:space:]]*=[[:space:]]*//p' "$envf" | tail -n1 | tr -d "\"' ")"
if [ "${#token}" -lt 20 ]; then
  new="$(head -c 96 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32)"
  if [ "${#new}" -lt 20 ]; then echo "Could not make a token; refusing to start a tunnel."; exit 1; fi
  if [ -f "$envf" ] && grep -q '^[[:space:]]*RHCSD_TOKEN' "$envf"; then
    sed -i "s/^[[:space:]]*RHCSD_TOKEN.*/RHCSD_TOKEN=$new/" "$envf"
  else
    printf '\nRHCSD_TOKEN=%s\n' "$new" >> "$envf"
  fi
  echo "RHCSD_TOKEN was missing or too short: wrote a new random one to $envf."
  echo "Restart rhcsd (./stop.sh then ./start.sh) and run this script again."
  exit 0
fi

command -v cloudflared >/dev/null 2>&1 || { echo "cloudflared is not installed. Run:  pkg install cloudflared"; exit 1; }

# rhcsd must already know the token (it reads .env only at start)
if command -v curl >/dev/null 2>&1; then
  h="$(curl -s -m 4 "http://127.0.0.1:$port/api/health")"
  case "$h" in
    *'"auth_required":true'*) ;;
    "") echo "rhcsd does not answer on port $port. Start it first (./start.sh)."; exit 1 ;;
    *) echo "rhcsd started before the token existed. Run ./stop.sh, ./start.sh, then this script again."; exit 1 ;;
  esac
fi

log="$dir/cloudflared.log"
: > "$log"
cloudflared tunnel --url "http://localhost:$port" --no-autoupdate > "$log" 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null' EXIT INT TERM

url=""
i=0
while [ $i -lt 60 ] && [ -z "$url" ]; do
  sleep 1
  kill -0 $pid 2>/dev/null || break
  url="$(grep -o 'https://[a-z0-9-]*\.trycloudflare\.com' "$log" | head -n1)"
  i=$((i + 1))
done
[ -n "$url" ] || { echo "No tunnel address appeared. See $log"; exit 1; }

echo "Tunnel address: $url"
echo
echo "Pairing link (contains your access token: send it only to yourself, delete it afterwards):"
echo "$pulse/#hub=$url&token=$token"
echo
echo "Keep this session open. Ctrl+C stops sharing."
wait $pid
