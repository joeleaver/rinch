package com.rinch;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.ContentValues;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Matrix;
import android.net.Uri;
import android.os.Bundle;
import android.os.Vibrator;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;
import android.location.Location;
import android.location.LocationListener;
import android.location.LocationManager;
import android.provider.MediaStore;
import android.view.ViewGroup;
import android.view.WindowManager;
import android.view.inputmethod.InputMethodManager;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.util.HashMap;

import android.media.ExifInterface;

/**
 * Rinch Activity — extends NativeActivity with platform service methods
 * callable from Rust via JNI (clipboard, IME, haptics, etc.).
 *
 * The NativeActivity base class handles the native library loading,
 * surface management, and lifecycle forwarding to the android-activity crate.
 */
public class RinchActivity extends NativeActivity {

    private InputMethodManager imm;
    private ClipboardManager clipboardManager;
    private Vibrator vibrator;
    private RinchInputView inputView;
    private Uri pendingPhotoUri;
    private SensorManager sensorManager;
    private final HashMap<Integer, SensorEventListener> activeSensors = new HashMap<>();
    private NotificationManager notificationManager;
    private int notificationId = 1;
    private LocationManager locationManager;
    private LocationListener locationListener;



    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
        clipboardManager = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
        vibrator = (Vibrator) getSystemService(Context.VIBRATOR_SERVICE);
        sensorManager = (SensorManager) getSystemService(Context.SENSOR_SERVICE);
        locationManager = (LocationManager) getSystemService(Context.LOCATION_SERVICE);
        notificationManager = (NotificationManager) getSystemService(Context.NOTIFICATION_SERVICE);

        // Add input proxy for proper InputConnection (CJK, autocomplete, etc.)
        // Must have non-zero size and be visible for IME to connect.
        inputView = new RinchInputView(this);
        ViewGroup.LayoutParams lp = new ViewGroup.LayoutParams(1, 1);
        addContentView(inputView, lp);
    }

    // ── Window-state replay across recreation (issue #475) ──────────────

    /**
     * The window state the app has asked for, mirrored where the window
     * cannot keep it.
     *
     * A configuration change — a rotation without {@code configChanges}, a
     * locale or font-size change, a theme switch — destroys this activity and
     * builds the next one around a <em>new</em> window, and window flags and
     * insets-controller state die with the old one. The native side's own
     * state (the keep-screen-on refcount, the app's idea of its bar
     * appearance) lives in the process and survives, so nothing over there
     * can notice the divergence: the app still believes it holds a flag the
     * new window has never seen. These statics survive exactly as long as
     * that native state does — the process — and
     * {@link #onAttachedToWindow()} replays them onto each new window.
     *
     * {@code null} means "the app never asked" and is never replayed, so a
     * fresh window keeps its platform defaults until the app says otherwise.
     * Recorded on whichever thread the native call arrives on, <em>before</em>
     * the UI-thread post that applies the value, so a recreation racing the
     * post still replays what was asked for; {@code volatile} is all the
     * coherence a single boxed write needs.
     *
     * One place owns the replay, on purpose: a setter added to this class
     * that writes window state without recording it here inherits the
     * original bug.
     */
    private static volatile Boolean requestedKeepScreenOn;
    private static volatile Boolean requestedLightStatusBars;
    private static volatile Boolean requestedLightNavigationBars;

    @Override
    public void onAttachedToWindow() {
        super.onAttachedToWindow();
        // Attach rather than onCreate: the insets controller the bar
        // appearance writes through does not exist until the decor view is
        // attached to the window manager. Going through the public setters
        // keeps one code path per property; each re-records the value it is
        // handed, which is the value already recorded.
        Boolean keepOn = requestedKeepScreenOn;
        if (keepOn != null) {
            setKeepScreenOn(keepOn);
        }
        Boolean lightStatus = requestedLightStatusBars;
        if (lightStatus != null) {
            setLightStatusBars(lightStatus);
        }
        Boolean lightNav = requestedLightNavigationBars;
        if (lightNav != null) {
            setLightNavigationBars(lightNav);
        }
    }

    // ── Safe Area Insets ─────────────────────────────────────────────────

    public int[] getSafeAreaInsets() {
        int top = 0, bottom = 0, left = 0, right = 0;

        android.view.View decorView = getWindow().getDecorView();
        android.view.WindowInsets insets = decorView.getRootWindowInsets();
        if (insets != null) {
            top = insets.getSystemWindowInsetTop();
            bottom = insets.getSystemWindowInsetBottom();
            left = insets.getSystemWindowInsetLeft();
            right = insets.getSystemWindowInsetRight();

            // Account for display cutout (notch)
            android.view.DisplayCutout cutout = insets.getDisplayCutout();
            if (cutout != null) {
                top = Math.max(top, cutout.getSafeInsetTop());
                bottom = Math.max(bottom, cutout.getSafeInsetBottom());
                left = Math.max(left, cutout.getSafeInsetLeft());
                right = Math.max(right, cutout.getSafeInsetRight());
            }
        } else {
            // Fallback: status bar height from resources
            int resourceId = getResources().getIdentifier("status_bar_height", "dimen", "android");
            if (resourceId > 0) {
                top = getResources().getDimensionPixelSize(resourceId);
            }
        }

        return new int[] { top, bottom, left, right };
    }

    // ── System Bar Appearance ───────────────────────────────────────────

    /**
     * Say whether the status bar sits over a light background.
     *
     * The clock, the battery and the signal icons are drawn by the system, not
     * by the app, and the system's default is light-on-dark. An app drawing
     * edge-to-edge behind a pale background therefore gets white glyphs on
     * cream until it says otherwise. {@code true} means "the background under
     * this bar is light, so draw its contents dark".
     *
     * @param light whether the bar's background is light
     */
    public void setLightStatusBars(boolean light) {
        requestedLightStatusBars = light; // survives recreation; replayed on attach
        runOnUiThread(() -> setBarAppearance(
            android.view.WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS, light));
    }

    /**
     * The same for the navigation bar's own contents — the three buttons, or
     * the gesture pill.
     *
     * Separate from {@link #setLightStatusBars(boolean)} on purpose: an app can
     * be pale under the status bar and dark under the navigation bar, and one
     * combined call would force it to lie about one end.
     *
     * @param light whether the bar's background is light
     */
    public void setLightNavigationBars(boolean light) {
        requestedLightNavigationBars = light; // survives recreation; replayed on attach
        runOnUiThread(() -> setBarAppearance(
            android.view.WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS, light));
    }

    /**
     * On the UI thread, and only there — window appearance is not thread-safe.
     *
     * From API 30 the appearance is a masked field on the window's
     * {@link android.view.WindowInsetsController}, so the mask names the one bit
     * being written and the other bar's bit is left alone. Below that it is the
     * decor view's system-UI visibility, in {@link #setLegacyBarAppearance}.
     */
    private void setBarAppearance(int appearance, boolean light) {
        android.view.View decorView = getWindow().getDecorView();
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R) {
            android.view.WindowInsetsController controller =
                decorView.getWindowInsetsController();
            if (controller != null) {
                controller.setSystemBarsAppearance(light ? appearance : 0, appearance);
            } else {
                // No window yet, so there is nothing to write the appearance to.
                // Say so — otherwise the bar silently stays light-on-dark.
                android.util.Log.w("rinch",
                    "system bar appearance dropped: the window has no insets controller yet");
            }
        } else {
            setLegacyBarAppearance(decorView, appearance, light);
        }
    }

    /**
     * The API 28–29 path: the same bit as a system-UI visibility flag,
     * read-modify-written so the other bar's flag survives.
     *
     * Both light-bar flags exist from API 26 and this crate's floor is 28, so
     * there is no third era to fall back to. The names are deprecated in favour
     * of the controller above and are reachable only on the versions that have
     * no controller, so the suppression lives here — the whole method is the
     * deprecated path — rather than on the callers, the same way
     * {@link #vibrate(long)} does it.
     */
    @SuppressWarnings("deprecation")
    private void setLegacyBarAppearance(android.view.View decorView, int appearance, boolean light) {
        int flag =
            appearance == android.view.WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                ? android.view.View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR
                : android.view.View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
        int flags = decorView.getSystemUiVisibility();
        decorView.setSystemUiVisibility(light ? (flags | flag) : (flags & ~flag));
    }

    // ── Keep Screen On ──────────────────────────────────────

    /**
     * Stop the display timeout while this window is in front, or let it run
     * again.
     *
     * {@code FLAG_KEEP_SCREEN_ON} is the right tool here and a wake lock is
     * not, even though both would keep the panel lit. The flag belongs to the
     * <em>window</em>: it holds the screen only while this window is the one
     * being shown — the system stops honouring it the moment the activity
     * stops, though the flag itself stays set until something clears it — and
     * it needs no permission. A {@code PowerManager.WakeLock} belongs to the
     * process, needs {@code WAKE_LOCK} in the manifest, and survives the app
     * being backgrounded — which is to say it survives every way an app has
     * of forgetting to release it, and the failure it produces is a phone that
     * quietly does not sleep for the rest of the day.
     *
     * Setting it costs nothing when it is already set: {@code addFlags} is an
     * or, {@code clearFlags} an and-not, so this is safe to call on every
     * change of whatever state the caller is mirroring rather than only on the
     * edges.
     *
     * @param keepOn whether the screen should stay on while this window shows
     */
    public void setKeepScreenOn(boolean keepOn) {
        requestedKeepScreenOn = keepOn; // survives recreation; replayed on attach
        // On the UI thread, and only there. Window flags are read by the view
        // hierarchy without a lock, and a flag written from the native frame
        // thread is a flag whose effect is decided by a race: on a good day the
        // next relayout picks it up, on a bad one the write lands in the middle
        // of one and does nothing at all. The same rule the IME and the bar
        // appearance above already follow, for the same reason.
        runOnUiThread(() -> {
            if (keepOn) {
                getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            } else {
                getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            }
        });
    }

    // Activity lifecycle (pause/resume) is delivered to the native run loop via
    // the android-activity glue's MainEvent::Pause/Resume, so no Java override —
    // and no direct JNI call — is needed here. Calling a native method from an
    // override would race the native thread that registers it (cold-start crash).

    // ── IME ─────────────────────────────────────────────────────────────

    public void showKeyboard() {
        runOnUiThread(() -> {
            if (imm != null && inputView != null) {
                inputView.setVisibility(android.view.View.VISIBLE);
                inputView.requestFocus();
                imm.restartInput(inputView);
                imm.showSoftInput(inputView, InputMethodManager.SHOW_FORCED);
            }
        });
    }

    /**
     * Tell the keyboard whether the field rinch has focused takes a line break.
     *
     * EditorInfo — where the Enter key's meaning is declared — is built once
     * per input session, so a keyboard that is already up will not notice a
     * move from a &lt;textarea&gt; to an &lt;input&gt; on its own: rinch moves
     * focus between its own fields without Android seeing anything, because
     * this one view holds focus throughout. Restarting the session is what
     * makes it ask again. Only when the value actually changed, so ordinary
     * field-to-field moves of the same kind cost nothing.
     *
     * Called before showKeyboard() when focus arrives, so the session the
     * keyboard opens is already the right kind; both post to this handler
     * queue, which keeps them in that order.
     */
    public void setInputMultiline(boolean multiline) {
        runOnUiThread(() -> {
            if (inputView == null || !inputView.setMultiline(multiline)) {
                return;
            }
            if (imm != null) {
                imm.restartInput(inputView);
            }
        });
    }

    public void hideKeyboard() {
        runOnUiThread(() -> {
            if (imm != null && inputView != null) {
                imm.hideSoftInputFromWindow(inputView.getWindowToken(), 0);
            }
        });
    }

    /**
     * Start the IME's input session over on the (unchanged) input view.
     *
     * Rinch moves focus between its own text fields without Android seeing
     * anything — there is one RinchInputView and it holds focus throughout —
     * so a keyboard part-way through composing a word would carry that word
     * into whichever field got focus next. This is called when rinch has
     * abandoned such a composition, and tells the keyboard to do the same.
     * showKeyboard() already does this for the field that raises the keyboard;
     * this is the same thing for a move between two fields, where the keyboard
     * is already up.
     */
    public void restartInput() {
        runOnUiThread(() -> {
            if (imm != null && inputView != null) {
                imm.restartInput(inputView);
            }
        });
    }

    // ── Clipboard ───────────────────────────────────────────────────────

    public void copyToClipboard(String text) {
        if (clipboardManager != null) {
            ClipData clip = ClipData.newPlainText("rinch", text);
            clipboardManager.setPrimaryClip(clip);
        }
    }

    public String pasteFromClipboard() {
        if (clipboardManager == null || !clipboardManager.hasPrimaryClip()) {
            return null;
        }
        ClipData clip = clipboardManager.getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0) {
            return null;
        }
        CharSequence text = clip.getItemAt(0).getText();
        return text != null ? text.toString() : null;
    }

    public boolean hasClipboardText() {
        return clipboardManager != null && clipboardManager.hasPrimaryClip();
    }

    // ── Haptics ─────────────────────────────────────────────────────────

    @SuppressWarnings("deprecation")
    public void vibrate(long ms) {
        if (vibrator != null && vibrator.hasVibrator()) {
            vibrator.vibrate(ms);
        }
    }

    // ── Activity Result Routing ─────────────────────────────────────────

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        String dataUri = null;
        if (data != null && data.getData() != null) {
            dataUri = data.getData().toString();
        } else if (pendingPhotoUri != null && resultCode == RESULT_OK) {
            dataUri = pendingPhotoUri.toString();
            pendingPhotoUri = null;
        }
        nativeOnActivityResult(requestCode, resultCode, dataUri);
    }

    private native void nativeOnActivityResult(int requestCode, int resultCode, String dataUri);

    // ── Permission Result Routing ───────────────────────────────────────

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        boolean allGranted = grantResults.length > 0;
        for (int result : grantResults) {
            if (result != PackageManager.PERMISSION_GRANTED) {
                allGranted = false;
                break;
            }
        }
        nativeOnPermissionsResult(requestCode, allGranted);
    }

    private native void nativeOnPermissionsResult(int requestCode, boolean allGranted);

    // ── File Picker (SAF) ───────────────────────────────────────────────

    public void openFilePicker(int requestCode) {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        startActivityForResult(intent, requestCode);
    }

    public void saveFilePicker(int requestCode, String fileName) {
        Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        if (fileName != null) {
            intent.putExtra(Intent.EXTRA_TITLE, fileName);
        }
        startActivityForResult(intent, requestCode);
    }

    // ── Image Picker / Camera ─────────────────────────────────────────

    public void openImagePicker(int requestCode) {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("image/*");
        startActivityForResult(intent, requestCode);
    }

    public void takePhoto(int requestCode) {
        try {
            ContentValues values = new ContentValues();
            values.put(MediaStore.Images.Media.DISPLAY_NAME,
                "rinch_" + System.currentTimeMillis() + ".jpg");
            values.put(MediaStore.Images.Media.MIME_TYPE, "image/jpeg");
            pendingPhotoUri = getContentResolver().insert(
                MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values);
            if (pendingPhotoUri != null) {
                Intent intent = new Intent(MediaStore.ACTION_IMAGE_CAPTURE);
                intent.putExtra(MediaStore.EXTRA_OUTPUT, pendingPhotoUri);
                startActivityForResult(intent, requestCode);
            }
        } catch (Exception e) {
            pendingPhotoUri = null;
        }
    }

    // ── Notifications ────────────────────────────────────────────────────

    private static final String CHANNEL_ID = "rinch_notifications";

    private void ensureNotificationChannel(int importance) {
        notificationManager.deleteNotificationChannel("rinch_default");
        NotificationChannel channel = notificationManager.getNotificationChannel(CHANNEL_ID);
        if (channel == null) {
            channel = new NotificationChannel(CHANNEL_ID, "Rinch Notifications", importance);
            notificationManager.createNotificationChannel(channel);
        }
    }

    public void showNotification(String title, String body, int importance) {
        ensureNotificationChannel(importance);
        android.app.Notification notification = new android.app.Notification.Builder(this, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(body)
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setAutoCancel(true)
            .build();
        notificationManager.notify(notificationId++, notification);
    }

    // ── Share ───────────────────────────────────────────────────────────

    public void shareText(String text) {
        Intent intent = new Intent(Intent.ACTION_SEND);
        intent.setType("text/plain");
        intent.putExtra(Intent.EXTRA_TEXT, text);
        startActivity(Intent.createChooser(intent, null));
    }

    public void shareImage(byte[] imageBytes, String text) {
        try {
            ContentValues values = new ContentValues();
            values.put(MediaStore.Images.Media.DISPLAY_NAME,
                "rinch_share_" + System.currentTimeMillis() + ".jpg");
            values.put(MediaStore.Images.Media.MIME_TYPE, "image/jpeg");
            Uri uri = getContentResolver().insert(
                MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values);
            if (uri == null) return;

            java.io.OutputStream os = getContentResolver().openOutputStream(uri);
            if (os != null) {
                os.write(imageBytes);
                os.close();
            }

            Intent intent = new Intent(Intent.ACTION_SEND);
            intent.setType("image/jpeg");
            intent.putExtra(Intent.EXTRA_STREAM, uri);
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            if (text != null) {
                intent.putExtra(Intent.EXTRA_TEXT, text);
            }
            startActivity(Intent.createChooser(intent, null));
        } catch (Exception e) {
            // sharing failed silently
        }
    }

    // ── Sensors ─────────────────────────────────────────────────────────

    public void startSensor(int sensorType, int delayUs) {
        Sensor sensor = sensorManager.getDefaultSensor(sensorType);
        if (sensor == null) return;
        // Unregister any existing listener for this type first; otherwise it
        // leaks (keeps firing) when startSensor is called twice for the same type.
        SensorEventListener existing = activeSensors.remove(sensorType);
        if (existing != null) sensorManager.unregisterListener(existing);
        SensorEventListener listener = new SensorEventListener() {
            @Override
            public void onSensorChanged(SensorEvent event) {
                nativeOnSensorChanged(sensorType, event.values, event.timestamp);
            }
            @Override
            public void onAccuracyChanged(Sensor s, int accuracy) {}
        };
        sensorManager.registerListener(listener, sensor, delayUs);
        activeSensors.put(sensorType, listener);
    }

    public void stopSensor(int sensorType) {
        SensorEventListener listener = activeSensors.remove(sensorType);
        if (listener != null) sensorManager.unregisterListener(listener);
    }

    private native void nativeOnSensorChanged(int sensorType, float[] values, long timestamp);

    // ── Location ───────────────────────────────────────────────────────

    @SuppressWarnings("MissingPermission")
    public void startLocationUpdates(long minTimeMs, float minDistanceM) {
        runOnUiThread(() -> {
            locationListener = new LocationListener() {
                @Override
                public void onLocationChanged(Location location) {
                    nativeOnLocationChanged(
                        location.getLatitude(), location.getLongitude(), location.getAltitude(),
                        location.getAccuracy(), location.getSpeed(), location.getBearing(),
                        location.getTime(),
                        location.getProvider() != null ? location.getProvider() : "unknown");
                }
            };

            try {
                if (locationManager.isProviderEnabled(LocationManager.GPS_PROVIDER)) {
                    locationManager.requestLocationUpdates(
                        LocationManager.GPS_PROVIDER, minTimeMs, minDistanceM, locationListener);
                }
            } catch (Exception e) { /* provider unavailable */ }

            try {
                if (locationManager.isProviderEnabled(LocationManager.NETWORK_PROVIDER)) {
                    locationManager.requestLocationUpdates(
                        LocationManager.NETWORK_PROVIDER, minTimeMs, minDistanceM, locationListener);
                }
            } catch (Exception e) { /* provider unavailable */ }
        });
    }

    public void stopLocationUpdates() {
        runOnUiThread(() -> {
            if (locationListener != null) {
                locationManager.removeUpdates(locationListener);
                locationListener = null;
            }
        });
    }

    private native void nativeOnLocationChanged(
        double lat, double lon, double alt, float accuracy,
        float speed, float bearing, long timestamp, String provider);

    // ── Image Reader (EXIF-aware) ─────────────────────────────────────

    public byte[] readImageUri(String uriString) {
        try {
            Uri uri = Uri.parse(uriString);

            // Read EXIF orientation
            int rotation = 0;
            try (InputStream exifStream = getContentResolver().openInputStream(uri)) {
                if (exifStream != null) {
                    ExifInterface exif = new ExifInterface(exifStream);
                    int orient = exif.getAttributeInt(
                        ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL);
                    switch (orient) {
                        case ExifInterface.ORIENTATION_ROTATE_90:  rotation = 90;  break;
                        case ExifInterface.ORIENTATION_ROTATE_180: rotation = 180; break;
                        case ExifInterface.ORIENTATION_ROTATE_270: rotation = 270; break;
                    }
                }
            }

            // Decode bitmap
            Bitmap bitmap;
            try (InputStream imageStream = getContentResolver().openInputStream(uri)) {
                bitmap = BitmapFactory.decodeStream(imageStream);
            }
            if (bitmap == null) return null;

            // Apply rotation if needed
            if (rotation != 0) {
                Matrix matrix = new Matrix();
                matrix.postRotate(rotation);
                Bitmap rotated = Bitmap.createBitmap(
                    bitmap, 0, 0, bitmap.getWidth(), bitmap.getHeight(), matrix, true);
                bitmap.recycle();
                bitmap = rotated;
            }

            // Encode to JPEG
            ByteArrayOutputStream baos = new ByteArrayOutputStream();
            bitmap.compress(Bitmap.CompressFormat.JPEG, 90, baos);
            bitmap.recycle();
            return baos.toByteArray();
        } catch (Exception e) {
            return null;
        }
    }

    // ── Content URI Reader ──────────────────────────────────────────────

    public byte[] readContentUri(String uriString) {
        try {
            Uri uri = Uri.parse(uriString);
            InputStream is = getContentResolver().openInputStream(uri);
            if (is == null) return null;
            ByteArrayOutputStream baos = new ByteArrayOutputStream();
            byte[] buffer = new byte[8192];
            int len;
            while ((len = is.read(buffer)) != -1) {
                baos.write(buffer, 0, len);
            }
            is.close();
            return baos.toByteArray();
        } catch (Exception e) {
            return null;
        }
    }

    // ── Content URI Writer ───────────────────────────────────────────────

    // The other half of the pair above, and the one `shareImage` above needed
    // only for a MediaStore JPEG it created itself — this one takes whatever
    // URI `saveFilePicker`'s `ACTION_CREATE_DOCUMENT` handed back, which can be
    // any document provider on the device, not just MediaStore. The idiom is
    // the same `openOutputStream` call `shareImage` makes, but that one never
    // reports failure to its caller because a failed share has nothing
    // downstream waiting on it; a failed save does, so this one returns
    // whether it worked rather than swallowing the exception silently.
    public boolean writeContentUri(String uriString, byte[] bytes) {
        try {
            Uri uri = Uri.parse(uriString);
            java.io.OutputStream os = getContentResolver().openOutputStream(uri);
            if (os == null) return false;
            os.write(bytes);
            os.close();
            return true;
        } catch (Exception e) {
            return false;
        }
    }
}
