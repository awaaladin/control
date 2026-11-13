// "Download the Android app" button. The APK is published by the GitHub Actions build to a
// release with a fixed tag, so this link never changes and always serves the newest build.

export const APK_URL = 'https://github.com/awaaladin/control/releases/download/android-latest/pulse.apk';

/** True when PULSE is running inside the Android app (its user agent carries "PulseApp"). */
export const inApp = (): boolean => /PulseApp/.test(navigator.userAgent);

export function AppDownload({ variant }: { variant: 'card' | 'pill' }) {
  if (inApp()) return null;

  if (variant === 'pill') {
    return (
      <a className="getapp-pill" href={APK_URL} download="pulse.apk" title="Download the PULSE Android app">
        ⬇ Get the app
      </a>
    );
  }

  return (
    <div className="getapp">
      <div className="getapp__text">
        <strong>Get the Android app</strong>
        <span>
          It finds your system by itself on your hotspot or Wi-Fi, so there is nothing to type, and it works where this
          secure website can't. After it downloads, open the file and allow "install unknown apps" once.
        </span>
      </div>
      <a className="btn btn--accent" href={APK_URL} download="pulse.apk">
        ⬇ Download the app (about 3 MB)
      </a>
    </div>
  );
}
