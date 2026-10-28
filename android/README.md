# Peegel for Android

Kotlin + Jetpack Compose app. Requires Android 10 (API 29) or newer.

## What it does

- **Computer → phone:** a foreground service keeps an encrypted connection to
  your computer. Anything copied there is put on the phone's clipboard
  automatically.
- **Phone → computer:** Android only lets the app on screen read the clipboard,
  so sending takes one tap:
  - the **Send clipboard** Quick Settings tile,
  - the **Send clipboard** button in Peegel's notification,
  - **Share → Send to computer** from any app,
  - the button in the app.

## Build

Requirements: JDK 17+ and the Android SDK (platform 37, build-tools 36).

```sh
cd android
echo "sdk.dir=$HOME/Android/Sdk" > local.properties   # if ANDROID_HOME isn't set
./gradlew assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

## Pairing

1. Open Peegel on the computer and go to **Devices**.
2. In the phone app, tap **Scan QR code** (or point the camera app at the code).
3. Allow notifications, and allow Peegel to run in the background. Samsung's
   battery saver otherwise closes the connection.
4. Optional: tap **Add the Quick Settings tile**.

## Samsung tips

- **Settings → Battery → Background usage limits:** make sure Peegel is not in
  *Sleeping apps* or *Deep sleeping apps*.
- Android shows a small "Peegel pasted from your clipboard" message when you
  send. That comes from the system and is expected.
