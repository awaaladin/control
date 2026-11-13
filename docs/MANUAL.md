# How to use rhcs

This is the user manual for **PULSE**, the rhcs control room, and the service
behind it. It explains how to open it, what every page and button does, how to
control your devices, and what to do when something goes wrong.

**rhcs** means **R**ust **H**ome **H**ardware **C**ontrol **S**ystem. **PULSE**
is the name of its web app.

---

## 1. Getting started

### Step 1: start the service on the laptop
Open PowerShell and run:
```powershell
cd D:\setup\rhcs
.\target\debug\rhcsd.exe
```
Leave this window open: it is the engine. It prints a line each time something
happens (router connected, phone connected, device joined…).
The first time, Windows may ask whether it can use the network. Choose
**Allow on private networks** so your phone can open the app too.

### Step 2: open the app
| On | Open this in the browser |
|---|---|
| The laptop | **http://localhost:7878** |
| Your phone, on the same Wi-Fi as the laptop | **http://LAPTOP-IP:7878**, e.g. `http://192.168.0.245:7878` on the router's Wi-Fi |
| Your phone, connected to the laptop by USB tethering | **http://10.150.40.57:7878** |

To find the laptop's IP, run `ipconfig` on the laptop and look under the
adapter you share with the phone.

### Step 3: check the top bar
The bar at the top tells you whether you're seeing real data:
| It shows | Meaning |
|---|---|
| **LIVE** (pulsing) | Connected; everything on screen is real and updating |
| **RECONNECTING** | The link to the laptop dropped for a moment; it's retrying on its own |
| **CONNECTION INTERRUPTED** | The home system can't be reached (laptop off, asleep, or not on this network). The app keeps trying and reconnects by itself |

PULSE never shows made-up data: if it can't reach your home system, it says so.

### Stopping
Press **Ctrl+C** in the service window. Stopping never changes anything on the
router or phone.

---

## 2. The pages

Use the menu on the left (on a phone: the tab bar at the bottom).

### 2.1 Signal: your 4G connection, live
What you see:
| Part | What it tells you |
|---|---|
| **Three glowing rings** | **RSRP** (signal strength), **SINR** (signal cleanliness), **RSRQ** (signal quality). Colour: green excellent, cyan good, amber fair, red poor. The rings gently pulse while data flows and turn grey with the word **Stale** if readings stop |
| **Down / Up** | Current download and upload speed through the router |
| **Timeline** | Moving graphs of the three signal values. Choose the window: **1 min, 10 min, 1 h, 24 h**. Touch or hover a graph to read exact values at that moment |
| **Throughput** | Download (above the line) and upload (below) over time |
| **Serving cell (Tower)** | Which tower and antenna the router is using: eNB (tower number), sector, PCI, band (e.g. B7 · 2600 MHz), channel, data used this session and this month |
| **Handovers** | Each time the router switches tower or band. Also shown as glowing markers on the graphs |

What the numbers mean:
| Value | Excellent | Good | Fair | Poor |
|---|---|---|---|---|
| RSRP (strength, dBm) | −80 or higher | −80 to −90 | −90 to −100 | below −100 |
| SINR (cleanliness, dB) | 20 or higher | 13 to 20 | 0 to 13 | below 0 |
| RSRQ (quality, dB) | −10 or higher | −10 to −15 | −15 to −20 | below −20 |

### 2.2 Aim mode: find the best spot for the router
Press **Aim mode** on the Signal page.
1. One giant ring shows **SINR**, the value that matters most for speed.
2. Switch **Tone** on to hear the signal: the pitch rises as it gets better. Useful when you can't watch the screen.
3. Move the router (or turn its antennas) slowly and wait 3–5 seconds at each position.
4. At a good position, press **Mark spot** and give it a name ("window", "shelf"). Your marked spots are listed with their readings so you can compare.
5. **Best so far** always shows the highest value reached.
6. Press **Close** to go back.

### 2.3 Phone: your phone as a second sensor
| Part | What it shows |
|---|---|
| **Battery** | Level, temperature, charging, and whether it's on USB power |
| **Phone LTE** | The phone's own signal rings: it has its own SIM and radio |
| **Phone vs router** | Side-by-side comparison: is the phone on the same tower? same band? which has the better signal and by how much |
| **Hotspot clients** | Every device connected to your phone's hotspot: name (if you've labelled it), maker, IP, MAC address |
| **USB / Wi-Fi badge** | How the laptop is talking to the phone right now |

Give a device on your hotspot a friendly name: add a line to `.env` such as
`KNOWN_SISTER_LAPTOP_MAC=aa:bb:cc:dd:ee:ff` and restart the service.

### 2.4 Network: everything connected
| Part | What it does |
|---|---|
| **Map view** | Each network the laptop is on is drawn as a constellation: the router in the centre, devices around it. Bigger dot = more open services |
| **Legend** | Colours: router, this laptop, phones, unknown devices |
| **View: map / table** | Switch to a sortable list |
| **Search devices** | Find a device by name, IP, MAC or maker |
| **Tap a device** | Opens its details: IP, MAC, maker, name, open services, what it announces about itself, first and last seen |
| **Scan** | Look for devices right now (asks "Scan the LAN now?" first). It also scans by itself every 5 minutes |
| **Alert on unknown devices** | Off by default. When on, the browser pops up a notification each time a device without a friendly name (see 2.3) joins the router's network or the phone's hotspot. Works on the laptop itself (`http://localhost:7878`); browsers block it on plain `http://` LAN addresses. The choice is remembered by that browser only |

When a device joins or leaves, it animates in or out and appears in the Log.

### 2.5 Audio: see sound
1. Press the button to **listen through this browser** and allow the microphone.
2. **Spectrum**: bars for bass (left) to treble (right), with peak markers.
3. **Level**: loudness meter.
4. **Oscilloscope**: the sound wave itself.
5. **Spectrogram**: a scrolling picture of sound over time.
6. **Freeze** stops the picture; **Beat glow** makes the whole app pulse on strong bass.

Put the device near the amplifier to watch your music.

### 2.6 Control: change things
| Section | Buttons | What happens |
|---|---|---|
| **Router** | **Re-login** | Starts a fresh login to the router. Use it after fixing the router password |
| | Reconnect, Disconnect, Network mode (Auto / LTE only) | Marked **not available yet**: these change router settings and will be enabled once tested safely |
| **Polling** | **Router refresh interval** slider (0.5–10 s), **Apply** | How often the router is read. Faster = smoother graphs, more router load |
| **LAN scan** | **Scan now** | Scan every connected network immediately; shows which networks are covered |
| **Source health** | (information) | One card each for Router, Phone and Scanner: online/offline, when it last worked, and an explanation if something is wrong |
| **Settings** | **Backend URL**, **Token**, **Save & reconnect** | Which laptop the app talks to, and the password for making changes from another device |
| | **Event sounds** | Soft chimes for handovers, disconnects and devices joining |

Every button that changes something first asks "Are you sure?", then shows a
message with the result.

### 2.7 Log: what happened and when
| Part | What it does |
|---|---|
| **Event list** | Every event, newest first: handovers, router/phone online/offline, devices joining/leaving, login problems |
| **Filter by kind** | Show only one kind of event |
| **Search events** | Find events by text (a device name, an IP…) |
| **Export signal CSV** | Download the signal history as a spreadsheet file |

### 2.8 Boost: lowest, steadiest ping for games
The laptop is often on more than one network at once: Wi-Fi to the router,
USB tethering to the phone, sometimes the phone's hotspot. **Game Boost** tests
each of them and tells you which one is best for online games (Free Fire and
the like).

1. Press **Run Game Boost**. It takes about 5 seconds.
2. Each connection gets a card that fills in live:

| Part | What it tells you |
|---|---|
| **Ping ring** | Typical time for a round trip to the internet, in milliseconds (ms). Green ≤ 50, cyan ≤ 90, amber ≤ 150, red above |
| **Jitter** | How much the ping jumps from one moment to the next. Under 8 ms feels smooth; above 30 ms you feel stutter even if ping is low |
| **Loss** | Pings that got no answer. Any loss causes stutter or "rubber-banding" |
| **Best / slow (p90)** | The fastest ping, and the ping that 1 in 10 pings is slower than |
| **Radio** | SINR and RSRP of the router or phone behind that connection, if known |
| **Graph** | Every ping of the test; red lines are pings that got lost |
| **In use now** | Windows is currently sending your internet through this connection |
| **Per target** | The same numbers for each test server |

3. The **Recommendation** box names the best connection, for example
   *"Use Phone (USB): 38 ms vs 71 ms, jitter 4 ms vs 19 ms"*, and explains why
   in plain words.
4. If the best connection isn't the one Windows uses, the box shows the
   command to switch, e.g.
   `powershell -ExecutionPolicy Bypass -File scripts\lan-only.ps1 -Interface "Wi-Fi"`.
   Run it yourself in an **admin** PowerShell in the rhcs folder; add `-Undo`
   to go back. rhcs never runs it for you and never changes network settings.
5. **Keep measuring** (switch at the top) repeats the test every 10 seconds
   until you press **Stop watching**: useful while moving the router.

Game Boost **cannot make the signal stronger** and cannot change how your
operator routes traffic. It tests general internet servers (Cloudflare, Google
and Quad9 by default), not the game's own servers: the game's own ping display
has the final word. To test other servers, set them in `.env`, e.g.
`BOOST_TARGETS=1.1.1.1:443,8.8.8.8:53` (host:port, comma separated).

Also on the page:
| Part | What it shows |
|---|---|
| **Tips for lower ping** | What else helps: Aim mode for the router, pausing heavy users (with a count of other devices on your networks), USB over hotspot, phone temperature |
| **Best time of day** | Average router SINR per hour of the day, best hours highlighted. Built from the last 24 h rhcsd has been running (it isn't saved when rhcsd stops) |

### 2.9 Towers: where your signal comes from
Every cell (tower antenna) the router or the phone has used is recorded, with
how long each stayed on it and the signal it got there. Records survive
restarts (they're saved in `data/towers.json`).

| Part | What it shows |
|---|---|
| **Map** | Home (teal dot), each tower, and a line from home to each tower. Line colour = average SINR seen on that tower; solid line = serving now; thickest line = cleanest tower. Needs an OpenCellID key |
| **Cleanest tower so far** | The cell with the best average SINR, and (with a key and home location) its distance and direction, e.g. "about 1.8 km to the north-east (47°)". Placing the router on the side of the house facing that direction usually helps |
| **Cells** | eNB (tower) and sector, PCI, band, time spent on it by router and phone, SINR average / best, RSRP average, distance and direction, when last seen. **router now / phone now** marks the current cell |
| **Handovers** | Every switch from one cell to another, by router or phone, with how long it stayed on the previous cell. Many short stays mean the router sits between two towers: try Aim mode |
| **Use this device's location** | Saves the position of the device you're holding as "home". Browsers only allow this on the laptop itself (`http://localhost:7878`) |

**Tower positions need a free OpenCellID key.** Without one the map is replaced
by instructions and nothing is guessed:
1. Sign up at **opencellid.org** and copy your access token.
2. Add `OPENCELLID_KEY=your-token` to `.env`.
3. Optional, for distance and direction: `HOME_LAT=` and `HOME_LON=` (decimal
   degrees, from any map app).
4. Restart rhcsd.

With a key, rhcsd sends the IDs of the cells it has seen (never your location)
to OpenCellID, one at a time, and keeps the answers. OpenCellID positions are
estimates collected by volunteers and can be off by a few hundred metres.

---

## 3. Controlling from your phone

1. Phone and laptop on the same network (the router's Wi-Fi, or USB tethering).
2. On the phone's browser open `http://LAPTOP-IP:7878`.
3. You can **view everything** right away.
4. To **make changes** from the phone (scan, re-login, settings):
   1. On the laptop, add `RHCSD_TOKEN=pick-a-long-secret` to `.env` and restart the service.
   2. On the phone: **Control → Settings → Token**, enter the same secret, **Save & reconnect**.
5. Tip: in Chrome on the phone, use **Add to Home screen** so PULSE opens like an app.

Who can do what:
| From | View | Change |
|---|---|---|
| The laptop | yes | yes |
| Phone/other device without the token | yes | no (you'll see "not allowed") |
| Phone/other device with the token | yes | yes |
| The internet | no | no |

Never forward port 7878 on a router to the internet.

---

## 4. When something goes wrong

PULSE explains problems in an **amber box**:
1. **What's wrong**, in plain words
2. **Why**, and what rhcs is already doing about it
3. **Numbered steps** to fix it
4. **Technical detail** (tap to open), the exact message, for asking for help

These boxes appear on the Signal page (router problems), the Phone page (phone
problems), and in **Control → Source health**.

### 4.1 Messages you may see and what to do
| Message | What it means | What to do | Fixes itself? |
|---|---|---|---|
| **Can't reach the router** | The laptop gets no answer from the router | Connect the laptop to the router (Wi-Fi or cable); check the router is on | Yes, reconnects within 5 s once reachable |
| **The router rejected the password** | Wrong admin password in `.env` | Fix `ROUTER_ADMIN_PASSWORD` in `.env`, restart, or press **Re-login** | No, waits for you (so the router can't lock) |
| **Login paused to protect the router** | Too many wrong passwords were tried | Log in once on the router's own page in a browser, then **Re-login** | No |
| **No router password configured** | `.env` has no router password | Add `ROUTER_ADMIN_PASSWORD` and restart | No |
| **The router answered in an unexpected format** | Different router firmware | Update rhcs and report the model | No |
| **Phone not connected** | No phone over USB or Wi-Fi | Plug in the phone; turn on USB debugging | Yes, checks every 5 s |
| **The phone hasn't allowed this computer yet** | USB debugging permission missing | Unlock the phone, tap **Allow** | Yes, after you allow |
| **The phone stopped responding** | The USB connection blipped | Check cable and port; unlock the phone | Usually yes |
| **ADB is not installed or not found** | The laptop can't find the phone tool | Install Android platform-tools | No |
| **Connection interrupted** (full screen) | The home system can't be reached: laptop off, asleep, service stopped, or you're on a different network. Shows when data last arrived | Start the service on the laptop; on a phone, join the same Wi-Fi and check Backend settings | Yes, reconnects as soon as it's reachable |
| **RECONNECTING** (top bar) | The live link blinked | Nothing | Yes |
| **Stale** on the rings | No new reading for a few seconds | Look for an amber box explaining why | Yes |
| **not allowed** | A change from another device without the token | Use the laptop, or set the token | n/a |
| **not available yet** | That control isn't enabled yet | n/a | n/a |
| **No internet link found** (Boost) | The laptop has no network with a gateway | Connect to the router or turn on USB tethering | Yes, run again once connected |
| **No answers: no internet through this link** (Boost) | That connection reaches its router/phone, but nothing beyond it (no data, no credit, or the router lost its 4G connection) | Check the router's or phone's mobile data | n/a |
| **Links are about equal** (Boost) | The best two connections are within a few ms | Nothing: switching won't be noticeable | n/a |
| **Tower locations need an OpenCellID key** (Towers) | No `OPENCELLID_KEY` in `.env` | See 2.9 | n/a |
| **Home location not set** (Towers) | No `HOME_LAT` / `HOME_LON` | Add them to `.env`, or press **Use this device's location** on the laptop | n/a |
| **not in OpenCellID** (Towers) | Nobody has mapped that cell yet | Nothing; it is retried weekly | Maybe |
| **lookup failed** (Towers) | No internet, wrong key, or daily limit reached; hover for the exact reason | Check the key | Retried every 10 min |
| **Microphone access was blocked** | The browser refused the microphone | Allow the microphone for this site in browser settings | n/a |

### 4.2 Common situations
| Situation | What happens | What to do |
|---|---|---|
| You unplug or move the router | Signal goes stale, amber box appears, event logged | Plug back in or reconnect: it resumes by itself |
| You unplug the phone | Phone page shows "Phone not connected" | Plug back in: it resumes by itself |
| Internet stops when you connect to the router | Windows is sending internet through a router with no data | Run once as admin: `scripts\lan-only.ps1 -Interface WiFi` |
| The phone's USB internet keeps dropping | The phone switches USB modes | Phone: Developer options → Default USB configuration → **USB tethering** |
| The laptop won't join the router's Wi-Fi | Weak signal where the laptop sits | Retry, or move closer |
| The USB flash drive disconnects | The service may stop | Reconnect the drive, start the service again |
| The laptop sleeps | Everything pauses | It continues when the laptop wakes |

---

## 5. Everyday recipes

| I want to… | Do this |
|---|---|
| Check if my internet is good right now | **Signal**: look at the ring colours |
| Find where to put the router | **Signal → Aim mode**, Tone on, move slowly, **Mark spot** |
| Get the best ping for a game | **Boost → Run Game Boost**, follow the recommendation |
| See which way the towers are | **Towers** (needs an OpenCellID key) |
| Know the best time to play | **Boost → Best time of day** |
| Know why it was slow last night | **Signal → Timeline → 24 h**, look for dips and handover markers |
| See who's on my Wi-Fi or hotspot | **Network** (router) and **Phone → Hotspot clients** |
| Spot a stranger | **Log**: filter by device joins; unknown makers stand out |
| Compare phone and router signal | **Phone → Phone vs router** |
| Fix "not logged in" | Correct the password in `.env`, then **Control → Re-login** |
| Make readings smoother | **Control → Polling**: lower the interval, **Apply** |
| Keep a record | **Log → Export signal CSV** |

---

## 6. What's coming

| Feature | Status |
|---|---|
| Reconnect mobile data / switch 4G mode from the app | Planned, once tested safely on the router |
| Phone app (Kotlin) with hotspot control and blocking devices | Planned |
| TV decoder and amplifier control | Planned via infrared |
| Open the app away from home | Planned via a private tunnel |
| Combining both routers and the phone into one faster connection | Planned |

---

## 7. Words used in the app

| Word | Meaning |
|---|---|
| **RSRP** | Signal strength from the tower |
| **SINR** | How clean the signal is compared to noise and interference: the best speed indicator |
| **RSRQ** | Signal quality |
| **RSSI** | Total received power, including noise |
| **eNB** | The tower's base station number |
| **Sector** | Which antenna on the tower faces you |
| **PCI** | The cell's short radio number |
| **Band (B3, B7…)** | Frequency range: B3 = 1800 MHz, B7 = 2600 MHz |
| **EARFCN** | The exact LTE channel |
| **TAC** | The tower's area code |
| **MCC / MNC** | Country and network code: 621/30 = MTN Nigeria |
| **Handover** | Switching from one tower or band to another |
| **Hotspot client** | A device using your phone's Wi-Fi hotspot |
| **Ping** | Time for a message to reach a server and come back, in ms |
| **Jitter** | How much the ping changes from one moment to the next |
| **Loss** | Messages that never got an answer |
| **Link** | One of the laptop's internet connections (router Wi-Fi, phone USB, phone hotspot) |
| **OpenCellID** | Free, open database of tower positions |
| **Bearing** | Direction in degrees, clockwise from north (90° = east) |
| **MAC** | A device's hardware address |
| **Stale** | No fresh reading for a few seconds |
| **Backend / service / rhcsd** | The program on the laptop that talks to your devices |
| **Token** | The password that allows changes from other devices |
| **ADB** | Android's tool for connecting a phone to a computer |
| **USB tethering** | Sharing the phone's internet over the USB cable |
