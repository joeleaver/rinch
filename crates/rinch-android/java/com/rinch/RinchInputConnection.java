package com.rinch;

import android.view.View;
import android.view.inputmethod.BaseInputConnection;

/**
 * Custom InputConnection that forwards IME text to Rust via JNI.
 *
 * Handles committed text, composing text, and deletions from the
 * Android soft keyboard (including CJK composing, autocomplete,
 * and swipe-to-type).
 *
 * This class keeps no state of its own. The composing region is mirrored on
 * the Rust side (`rinch::shell::android_ime::ImeComposition`), because the two
 * things that end a composition without the IME's involvement — the focused
 * field changing, and the app being backgrounded — are only visible there.
 * Two mirrors could disagree; one cannot.
 */
public class RinchInputConnection extends BaseInputConnection {

    public RinchInputConnection(View targetView, boolean fullEditor) {
        super(targetView, fullEditor);
    }

    @Override
    public boolean commitText(CharSequence text, int newCursorPosition) {
        // Forwarded even when empty: committing the empty string is how an IME
        // throws a composing region away, and the Rust side has to hear about
        // it to stop drawing the composition.
        nativeCommitText(text == null ? "" : text.toString());
        return true;
    }

    @Override
    public boolean setComposingText(CharSequence text, int newCursorPosition) {
        // Every call carries the whole composing region, not the latest
        // keystroke — which is `ImeEvent::Preedit`'s contract exactly, so it is
        // forwarded verbatim. This used to return true and forward nothing,
        // which is why autocorrect, swipe-to-type and CJK had no inline
        // preview: the first thing rinch heard about a word was its commit.
        nativeSetComposingText(text == null ? "" : text.toString(), newCursorPosition);
        return true;
    }

    @Override
    public boolean finishComposingText() {
        // In an EditText the composing text is already in the buffer and this
        // only drops the spans. Rinch keeps the preedit outside the field's
        // value, so finishing has to commit it — Gboard ends a composition this
        // way when the user accepts a word by tapping elsewhere, and treating
        // it as a discard would delete what they typed.
        nativeFinishComposingText();
        return true;
    }

    @Override
    public boolean setComposingRegion(int start, int end) {
        // Deliberately unsupported, rather than accidentally so. It asks for
        // text already in the field to become the composing region, and this
        // connection reports no surrounding text at all (rinch owns the value,
        // so `getEditable()` is permanently empty), which leaves an IME no way
        // to have chosen a meaningful range and this side no way to honour one.
        // BaseInputConnection would no-op it against that empty editable; this
        // override says so out loud. `true` keeps the connection alive — the
        // documented meaning of `false` here is "no longer valid", which would
        // be a different and worse lie.
        return true;
    }

    @Override
    public boolean deleteSurroundingText(int beforeLength, int afterLength) {
        nativeDeleteSurrounding(beforeLength, afterLength);
        return true;
    }

    @Override
    public boolean sendKeyEvent(android.view.KeyEvent event) {
        // Let NativeActivity's input queue handle raw key events
        return super.sendKeyEvent(event);
    }

    // ── JNI native methods (implemented in rinch-android/src/ime.rs) ───
    //
    // One queue, in call order. A composition and the commit that ends it are a
    // sequence, and separate queues could not keep them in order.

    private static native void nativeCommitText(String text);
    private static native void nativeSetComposingText(String text, int newCursorPosition);
    private static native void nativeFinishComposingText();
    private static native void nativeDeleteSurrounding(int before, int after);
}
