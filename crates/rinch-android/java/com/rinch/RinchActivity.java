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

    public void hideKeyboard() {
        runOnUiThread(() -> {
            if (imm != null && inputView != null) {
                imm.hideSoftInputFromWindow(inputView.getWindowToken(), 0);
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
}
