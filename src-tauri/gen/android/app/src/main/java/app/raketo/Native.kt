package app.raketo

/** Callbacks into the Rust side (src-tauri/src/android.rs). */
object Native {
  @JvmStatic external fun onData(data: ByteArray)
  @JvmStatic external fun onClosed()
}
