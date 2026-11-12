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
| **DEMO** (amber badge) | Simulated data for trying the app; nothing is real |

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
| | **Demo mode** | Switch to simulated data (always marked DEMO) |
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
| **rhcsd isn't answering** (full screen) | The service isn't running or can't be reached | Start it on the laptop; from a phone, check the backend URL | Yes, connects as soon as it's up |
| **RECONNECTING** (top bar) | The live link blinked | Nothing | Yes |
| **Stale** on the rings | No new reading for a few seconds | Look for an amber box explaining why | Yes |
| **not allowed** | A change from another device without the token | Use the laptop, or set the token | n/a |
| **not available yet** | That control isn't enabled yet | n/a | n/a |
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
| Know why it was slow last night | **Signal → Timeline → 24 h**, look for dips and handover markers |
| See who's on my Wi-Fi or hotspot | **Network** (router) and **Phone → Hotspot clients** |
| Spot a stranger | **Log**: filter by device joins; unknown makers stand out |
| Compare phone and router signal | **Phone → Phone vs router** |
| Fix "not logged in" | Correct the password in `.env`, then **Control → Re-login** |
| Make readings smoother | **Control → Polling**: lower the interval, **Apply** |
| Keep a record | **Log → Export signal CSV** |
| Try the app without devices | **Control → Settings → Demo mode** |

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
| **MAC** | A device's hardware address |
| **Stale** | No fresh reading for a few seconds |
| **Backend / service / rhcsd** | The program on the laptop that talks to your devices |
| **Token** | The password that allows changes from other devices |
| **ADB** | Android's tool for connecting a phone to a computer |
| **USB tethering** | Sharing the phone's internet over the USB cable |
