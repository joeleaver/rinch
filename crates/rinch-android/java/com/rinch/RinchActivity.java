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
import java.io.OutputStream;
import java.util.ArrayList;
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

        // The intent that started us — a share, a deep link, a shortcut. Stored
        // rather than delivered: at this point in a cold start the native
        // thread may not have registered nativeOnIncomingIntent yet. See the
        // Incoming Intents section below. A non-null savedInstanceState says
        // this is a rebuild rather than a launch, which is what makes the
        // difference between the launch intent and a stale copy of one.
        queueIntent(getIntent(), savedInstanceState != null);
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

    // ── Viewport Size ────────────────────────────────────────────────────

    /**
     * The size of the window this activity draws into, in physical pixels, as
     * {@code { width, height }}.
     *
     * This is the hole {@link #getSafeAreaInsets()} left. An app can already ask
     * how much of the screen it is not allowed to draw in, what the panel's
     * refresh rate is and how dense its pixels are — but not how big it is.
     *
     * <p>To be clear about what this is <em>not</em>: it is not a repair to the
     * shell's layout. The native side has always laid out against the real
     * window — it reads {@code ANativeWindow_getWidth}/{@code Height} at
     * {@code InitWindow} and {@code WindowResized} — so anything the DOM lays
     * out is already the right size. What was missing is that <em>app</em> code
     * had no way to ask. A page the app rasterises itself, at a pixel width, and
     * hands back as an image is not laid out by the DOM and gets none of that
     * for free. SetListArray's card K31 is what found it: it renders its pages
     * at the 393-pixel canvas its designs were drawn on, and on a moto g stylus
     * 5G — 1080 physical pixels at density 400, so 432 logical — every page came
     * out 393 wide with a strip of backdrop down each side. Nothing was broken;
     * the app had no way to find out it was drawing at the wrong size, and it
     * was wrong by a different amount on every handset.
     *
     * <p>Physical pixels, like {@link #getSafeAreaInsets()}, because that is
     * what Android measures in and the caller already has to divide by the
     * density to get anywhere. Divide by {@code densityDpi / 160} — the same
     * scale factor the shell derives at {@code InitWindow} — to get the logical
     * pixels a stylesheet is written in.
     *
     * <p>Two paths. From API 30 the window's own metrics are the answer:
     * {@code getCurrentWindowMetrics().getBounds()} is the bounds of *this
     * window*, system bars included, which is what a full-screen activity
     * actually occupies and what the {@code ANativeWindow} is. Below that the
     * call does not exist, so it falls back to the display metrics the resources
     * carry, which are the app-usable <em>display</em> size — very slightly
     * shorter than the window on a device with decorations, and identical in
     * width <em>in a single-window session</em>.
     *
     * <p>That last qualifier is the assumption this method makes and cannot
     * check: <strong>multi-window and split-screen are assumed away below API
     * 30</strong>. A side-by-side split disagrees in width, not merely in
     * height, so the fallback answers the wrong number there. Above API 30 there
     * is no such gap — window metrics are window metrics.
     */
    public int[] getViewportSize() {
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R) {
            android.graphics.Rect bounds =
                getWindowManager().getCurrentWindowMetrics().getBounds();
            return new int[] { bounds.width(), bounds.height() };
        }
        android.util.DisplayMetrics metrics = getResources().getDisplayMetrics();
        return new int[] { metrics.widthPixels, metrics.heightPixels };
    }

    // ── IME Inset ────────────────────────────────────────────────────────

    /**
     * How much of the bottom edge the soft keyboard is covering right now, in
     * physical pixels, or {@code -1} when the window has no insets to report
     * yet.
     *
     * Deliberately *not* folded into {@link #getSafeAreaInsets()}, even though
     * both are bottom insets and both are read the same way. The safe area is
     * a property of the hardware — the gesture bar and the notch are where they
     * are for the life of the process, which is why the app reads it once at
     * mount and never again. The keyboard comes and goes several times a
     * minute. One call returning both would either force the cheap question to
     * be asked at the expensive question's rate, or quietly make the safe area
     * change under a caller that was promised it would not.
     *
     * <p>Two paths again. From API 30 {@code WindowInsets.Type.ime()} is the
     * framework's own answer and is reported whether or not the window fits
     * system windows itself. Below that there is no ime type at all, and the
     * keyboard arrives folded into the deprecated system-window insets: the
     * bottom system-window inset is the navigation bar plus the keyboard, and
     * the bottom *stable* inset is the navigation bar alone, so the difference
     * is the keyboard.
     *
     * <p><strong>That legacy subtraction is very likely to report nothing on
     * this shell, and has never been run.</strong> The difference is non-zero
     * only while the window is <em>being resized for the IME</em>, and the
     * manifest declares no {@code windowSoftInputMode}. So on API 28-29 the
     * likely answer with the keyboard up is {@code 0} — which the Rust side
     * decodes as "the keyboard is down", the instruction to put a footer back.
     * The handset this was built against is a moto g stylus 5G on <em>SDK
     * 33</em>, so only the API 30+ branch has ever executed: the preceding
     * sentence is reasoning about the platform, not a measurement. Treat the
     * inset as API 30+ until someone with an API 28-29 device says otherwise.
     *
     * <p>This is a polled getter and not a callback, so a caller that wants to
     * animate with the keyboard has to ask on each frame it cares about — and
     * since the Android loop learned to sleep when nothing moves, it has to
     * arrange for those frames to exist (see {@code display::ime_inset} on the
     * Rust side, which names the mechanism). The Java-side push route —
     * {@code setDecorFitsSystemWindows(false)} plus an
     * {@code OnApplyWindowInsetsListener} — is left for whoever needs it:
     * turning decor fitting off changes where the window lays out underneath
     * the {@code ANativeWindow} the native shell is drawing into, which is a
     * change to every rinch app's geometry rather than an addition to it. It is
     * not the only push route, though; {@code MainEvent.ContentRectChanged}
     * already reaches the native loop and nothing handles it yet.
     */
    public int getImeInset() {
        android.view.WindowInsets insets = getWindow().getDecorView().getRootWindowInsets();
        if (insets == null) {
            return -1;
        }
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R) {
            return insets.getInsets(android.view.WindowInsets.Type.ime()).bottom;
        }
        return legacyImeInset(insets);
    }

    /**
     * The API 28–29 path, in its own method for the same reason
     * {@link #setLegacyBarAppearance} is: the whole body is deprecated calls
     * that are only reachable on the versions which have nothing else, so the
     * suppression belongs here rather than on the caller.
     */
    @SuppressWarnings("deprecation")
    private int legacyImeInset(android.view.WindowInsets insets) {
        return Math.max(0, insets.getSystemWindowInsetBottom() - insets.getStableInsetBottom());
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

    // ── Incoming Intents ─────────────────────────────────────────────────
    //
    // The other half of Share above. An app whose manifest declares an
    // <intent-filter> for ACTION_SEND is a share target from the moment it is
    // installed: it appears in every chooser, the user picks it, and Android
    // launches it with the payload attached. Nothing here ever read that
    // payload, so being launched and being told why were two different things
    // and only the first one worked.
    //
    // Nothing in this section calls a native from a lifecycle override. That is
    // the same rule the pause/resume comment further up states, and it is the
    // whole reason this is a queue rather than a straight call: onCreate runs
    // on the UI thread at a moment when the native thread may not have reached
    // RegisterNatives, and a native that is not registered yet is an
    // UnsatisfiedLinkError that force-finishes the activity on cold start. So
    // onCreate only stores, and Rust calls flushPendingIntents() at the end of
    // its own init to say the far side is ready. After that the queue is a
    // formality and onNewIntent goes straight through.

    /** Intents seen before the native side said it was ready to be called. */
    private final ArrayList<Intent> pendingIntents = new ArrayList<>();

    /**
     * Whether Rust has called {@link #flushPendingIntents()}. Guarded by
     * {@link #pendingIntents}, because onNewIntent reads it on the UI thread
     * while flushPendingIntents writes it on the native thread.
     */
    private boolean nativeIntentsReady = false;

    /** Marks an Intent this activity has already handed over. See {@link #queueIntent}. */
    private static final String EXTRA_RINCH_CONSUMED = "com.rinch.intentConsumed";

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        // getIntent() answers with the *starting* intent until you say
        // otherwise; the framework does not do this for you. Anything that
        // later reads getIntent() — including a recreation of this activity —
        // should see what actually arrived, and the consumed marker below is
        // only useful if it is attached to the copy that gets remembered.
        setIntent(intent);
        queueIntent(intent, false);
    }

    /**
     * Take an intent for delivery to Rust, or decide it is not worth one.
     *
     * <p><b>The stale re-delivery guard.</b> {@code getIntent()} does not mean
     * "the intent that just arrived", it means "the intent this activity was
     * last started with", and Android hands the same one back more than once:
     *
     * <ul>
     *   <li>Rotate the screen, or let the system reclaim the process and then
     *       restore the task from Recents, and the activity is destroyed and
     *       built again <em>with the original intent</em>. A share consumed
     *       before the rotation arrives a second time after it.
     *   <li>With {@code launchMode="singleTask"} the task is reused rather than
     *       rebuilt, so one Intent object can outlive several trips through the
     *       launcher inside a single process.
     * </ul>
     *
     * <p>Either way the user made one share and the app acts on it twice, which
     * for a share target is a duplicate import with no undo — and it is easy to
     * miss, because the happy path (launch, share, done) never shows it. Two
     * guards, because neither one covers the other's case:
     *
     * <ul>
     *   <li>{@code recreated} is {@code savedInstanceState != null} in
     *       onCreate. A non-null bundle means an earlier incarnation of this
     *       activity ran, and whatever {@code getIntent()} is about to return
     *       was already offered to it. This is the one that catches rotation
     *       and a process the system killed and restored. "Offered" and not
     *       "acted on": an app killed in the half-second between the share
     *       arriving and the frame loop draining it loses that share. That is
     *       the deliberate half of the trade — a share silently dropped once in
     *       a blue moon is recoverable by sharing again, and a share silently
     *       imported twice on every rotation is not.
     *   <li>{@link #EXTRA_RINCH_CONSUMED} is put on the Intent itself, so the
     *       same object handed back a second time within one process is
     *       recognisable. It does not survive process death — the marker goes
     *       on this process's copy and the system re-supplies its own — which
     *       is exactly the gap the bundle check fills.
     * </ul>
     *
     * @param intent    the intent to consider; null is tolerated
     * @param recreated whether this activity is being rebuilt rather than launched
     */
    private void queueIntent(Intent intent, boolean recreated) {
        if (intent == null) {
            return;
        }
        try {
            if (recreated || intent.getBooleanExtra(EXTRA_RINCH_CONSUMED, false)) {
                return;
            }
            // The launcher's own intent: ACTION_MAIN with nothing attached.
            // Every cold start produces one and there is nothing in it to
            // deliver. The payload check is what keeps an app shortcut — also
            // ACTION_MAIN, but carrying a data URI saying which shortcut — from
            // being swallowed with it. Rust filters again on the way into its
            // queue and that copy is the authoritative one (it has the tests);
            // this exists so a launcher tap does not grow the list below on an
            // app that has not finished starting yet.
            String action = intent.getAction();
            boolean bareLaunch = !carriesPayload(intent)
                && (action == null || action.isEmpty() || Intent.ACTION_MAIN.equals(action));
            if (bareLaunch) {
                return;
            }
            intent.putExtra(EXTRA_RINCH_CONSUMED, true);
        } catch (Exception e) {
            // Reading extras unparcels data written by whichever app sent the
            // share, so a malformed or unresolvable Parcelable throws here
            // rather than anywhere useful. An intent we cannot even inspect is
            // not one we can deliver.
            return;
        }

        boolean deliverNow;
        synchronized (pendingIntents) {
            pendingIntents.add(intent);
            deliverNow = nativeIntentsReady;
        }
        if (deliverNow) {
            deliverPendingIntents();
        }
    }

    /**
     * Whether an intent carries anything beyond its action. Mirrors
     * {@code IncomingIntent::has_payload} on the Rust side.
     */
    private static boolean carriesPayload(Intent intent) {
        return intent.getType() != null
            || intent.getCharSequenceExtra(Intent.EXTRA_TEXT) != null
            || intent.hasExtra(Intent.EXTRA_STREAM)
            || intent.getData() != null;
    }

    /**
     * Rust says its natives are registered; deliver everything held back and
     * stop holding anything back.
     *
     * <p>Called from the native thread at the end of {@code rinch_android::init}
     * — never from Java — which is the handshake that makes the whole queue
     * safe. Java never calls a native until Rust has said so.
     */
    public void flushPendingIntents() {
        synchronized (pendingIntents) {
            nativeIntentsReady = true;
        }
        deliverPendingIntents();
    }

    /** Push whatever is queued through the native, oldest first. */
    private void deliverPendingIntents() {
        ArrayList<Intent> batch;
        synchronized (pendingIntents) {
            if (pendingIntents.isEmpty()) {
                return;
            }
            batch = new ArrayList<>(pendingIntents);
            pendingIntents.clear();
        }
        // Outside the lock. The native side takes a lock of its own and this
        // one is also taken from the UI thread by onNewIntent; holding both at
        // once, in an order nobody controls, is how a share freezes an app.
        for (int i = 0; i < batch.size(); i++) {
            pushIntent(batch.get(i));
        }
    }

    /** Flatten one intent and hand it to Rust. */
    private void pushIntent(Intent intent) {
        try {
            String action = intent.getAction();

            // getCharSequenceExtra, not getStringExtra: EXTRA_TEXT is declared
            // as a CharSequence, and a share from a rich-text field really does
            // arrive as a Spanned — on which getStringExtra returns null rather
            // than the text, so the share looks empty for no visible reason.
            CharSequence text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT);

            ArrayList<String> streams = new ArrayList<>();
            if (Intent.ACTION_SEND_MULTIPLE.equals(action)) {
                ArrayList<Uri> uris = intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM);
                if (uris != null) {
                    for (Uri uri : uris) {
                        if (uri != null) {
                            streams.add(uri.toString());
                        }
                    }
                }
            } else {
                Uri uri = intent.getParcelableExtra(Intent.EXTRA_STREAM);
                if (uri != null) {
                    streams.add(uri.toString());
                }
            }

            Uri data = intent.getData();
            nativeOnIncomingIntent(
                action,
                intent.getType(),
                text == null ? null : text.toString(),
                streams.toArray(new String[0]),
                data == null ? null : data.toString());
        } catch (Exception e) {
            // Same reasoning as queueIntent: the contents came from another
            // app. Losing one malformed share is better than taking the
            // process down on the native thread.
        }
    }

    /**
     * Safe from any thread: the Rust side of this only pushes onto a mutex-held
     * queue and wakes the frame loop, and the handler runs later, on the main
     * thread, out of the loop's own drain.
     */
    private native void nativeOnIncomingIntent(
        String action, String mimeType, String text, String[] streamUris, String dataUri);

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

    // try-with-resources, matching the writer below: the bare `is.close()`
    // this used to end with does not run when `read` throws, leaking the
    // provider's file descriptor for the life of the process.
    public byte[] readContentUri(String uriString) {
        try {
            Uri uri = Uri.parse(uriString);
            try (InputStream is = getContentResolver().openInputStream(uri)) {
                if (is == null) return null;
                ByteArrayOutputStream baos = new ByteArrayOutputStream();
                byte[] buffer = new byte[8192];
                int len;
                while ((len = is.read(buffer)) != -1) {
                    baos.write(buffer, 0, len);
                }
                return baos.toByteArray();
            }
        } catch (Exception e) {
            android.util.Log.w("rinch", "readContentUri failed: " + uriString, e);
            return null;
        }
    }

    // ── Content URI Writer ───────────────────────────────────────────────

    // The other half of the reader above. `shareImage` also writes through a
    // content URI, but only to a MediaStore JPEG it created itself; this one
    // takes whatever URI `saveFilePicker`'s ACTION_CREATE_DOCUMENT handed
    // back, which can be any document provider on the device. It reports
    // success rather than swallowing the exception, because a failed save has
    // a caller waiting on it where a failed share does not.
    //
    // Mode "wt", NOT the "w" that the one-argument openOutputStream(uri)
    // defaults to. Truncation under "w" is explicitly undefined — the platform
    // javadoc for this call says "the exact implementation of these may differ
    // for each Provider implementation - for example, 'w' may or may not
    // truncate" — and the framework's own helper leaves it off:
    // FileUtils.translateModeStringToPosix maps a mode starting with "w" to
    // O_WRONLY | O_CREAT and adds O_TRUNC only when the string contains 't'.
    // So on any provider using it, writing 6KB over an existing 10KB document
    // leaves the last 4KB of the old file in place and still reports success:
    // silent corruption in the save path, with the caller told it worked.
    // "wt" asks for truncation explicitly and is in this call's documented
    // mode set ("r", "w", "wt", "wa", "rw", "rwt").
    //
    // This is not only about re-saving to a URI the app kept. A provider is
    // free to return an existing document when the user picks a name already
    // taken, and write_content_uri is public API that a caller may point at
    // any content URI it holds — so "ACTION_CREATE_DOCUMENT made it, therefore
    // it is empty" is not an assumption this can rest on.
    //
    // try-with-resources, so a throwing write still closes the stream. An
    // OutputStream left open holds buffered bytes that never reach the
    // document and can leave the provider's file locked or half-written.
    public boolean writeContentUri(String uriString, byte[] bytes) {
        try {
            Uri uri = Uri.parse(uriString);
            try (OutputStream os = getContentResolver().openOutputStream(uri, "wt")) {
                if (os == null) return false;
                os.write(bytes);
                os.flush();
                return true;
            }
        } catch (Exception e) {
            android.util.Log.w("rinch", "writeContentUri failed: " + uriString, e);
            return false;
        }
    }
}
