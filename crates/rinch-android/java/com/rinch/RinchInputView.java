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

    /**
     * Whether the field rinch currently has focused takes a line break — i.e.
     * whether it is a &lt;textarea&gt;. One view serves every rinch field in
     * turn, so this is the focused field's property, not the view's, and it is
     * pushed here whenever focus moves (RinchActivity.setInputMultiline).
     *
     * Written and read on the UI thread only: the push hops there via
     * runOnUiThread, and onCreateInputConnection is called there.
     */
    private boolean multiline = false;

    public RinchInputView(Context context) {
        super(context);
        setFocusable(true);
        setFocusableInTouchMode(true);
    }

    /**
     * Set the focused field's kind. Returns true when it changed, which is
     * when the IME's input session has to be restarted to see it.
     */
    public boolean setMultiline(boolean value) {
        if (multiline == value) {
            return false;
        }
        multiline = value;
        return true;
    }

    @Override
    public boolean onCheckIsTextEditor() {
        return true;
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
        // The Enter key is declared here, once per input session, and it is
        // declared per field. Without TYPE_TEXT_FLAG_MULTI_LINE the keyboard
        // draws an action key (↵/Go/Done) whose press *ends* the session
        // instead of typing anything — so a <textarea> could not be given a
        // line break from the soft keyboard at all, and pressing the key
        // dismissed the keyboard. IME_FLAG_NO_ENTER_ACTION says the same thing
        // from the other side: this editor has no action for Enter to run.
        //
        // A single-line <input> keeps exactly what it had. IME_ACTION_UNSPECIFIED
        // is 0, so it contributes nothing to the OR and is kept only because it
        // says out loud that rinch declares no editor action there either.
        outAttrs.inputType = EditorInfo.TYPE_CLASS_TEXT
                | EditorInfo.TYPE_TEXT_FLAG_AUTO_CORRECT
                | (multiline ? EditorInfo.TYPE_TEXT_FLAG_MULTI_LINE : 0);
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI
                | EditorInfo.IME_FLAG_NO_FULLSCREEN
                | (multiline ? EditorInfo.IME_FLAG_NO_ENTER_ACTION
                             : EditorInfo.IME_ACTION_UNSPECIFIED);
        return new RinchInputConnection(this, true);
    }
}
