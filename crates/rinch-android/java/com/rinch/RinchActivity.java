package com.rinch;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.ContentValues;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Bundle;
import android.os.Vibrator;
import android.provider.MediaStore;
import android.view.ViewGroup;
import android.view.inputmethod.InputMethodManager;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;

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



    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        imm = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
        clipboardManager = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
        vibrator = (Vibrator) getSystemService(Context.VIBRATOR_SERVICE);

        // Add input proxy for proper InputConnection (CJK, autocomplete, etc.)
        // Must have non-zero size and be visible for IME to connect.
        inputView = new RinchInputView(this);
        ViewGroup.LayoutParams lp = new ViewGroup.LayoutParams(1, 1);
        addContentView(inputView, lp);
    }

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
        Intent intent = new Intent(MediaStore.ACTION_IMAGE_CAPTURE);
        if (intent.resolveActivity(getPackageManager()) != null) {
            ContentValues values = new ContentValues();
            values.put(MediaStore.Images.Media.DISPLAY_NAME,
                "rinch_" + System.currentTimeMillis() + ".jpg");
            values.put(MediaStore.Images.Media.MIME_TYPE, "image/jpeg");
            pendingPhotoUri = getContentResolver().insert(
                MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values);
            if (pendingPhotoUri != null) {
                intent.putExtra(MediaStore.EXTRA_OUTPUT, pendingPhotoUri);
                startActivityForResult(intent, requestCode);
            }
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
