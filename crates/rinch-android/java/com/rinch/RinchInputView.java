package com.rinch;

import android.content.Context;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;

/**
 * Invisible proxy view that provides an InputConnection for the soft keyboard.
 *
 * NativeActivity's internal SurfaceView returns null from onCreateInputConnection,
 * so the IME falls back to raw key events. This view overrides that: when focused,
 * it returns our RinchInputConnection which handles composing text, CJK input,
 * autocomplete, and swipe-to-type.
 *
 * Added as a 0x0 overlay in onCreate — doesn't interfere with rendering.
 */
public class RinchInputView extends View {

    public RinchInputView(Context context) {
        super(context);
        setFocusable(true);
        setFocusableInTouchMode(true);
    }

    @Override
    public boolean onCheckIsTextEditor() {
        return true;
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
        outAttrs.inputType = EditorInfo.TYPE_CLASS_TEXT
                | EditorInfo.TYPE_TEXT_FLAG_AUTO_CORRECT;
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI
                | EditorInfo.IME_FLAG_NO_FULLSCREEN
                | EditorInfo.IME_ACTION_UNSPECIFIED;
        return new RinchInputConnection(this, true);
    }
}
