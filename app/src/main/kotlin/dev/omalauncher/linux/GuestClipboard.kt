package dev.omalauncher.linux

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.util.Log
import java.io.File
import java.io.FileOutputStream
import java.io.OutputStream

private const val TAG = "clip"

/** Longest side of a pasted image: enough for an agent to read a screenshot, small enough to encode fast. */
private const val MAX_SIDE = 2048

/**
 * Android's clipboard for the guest's `wl-paste`, images included: the Wayland
 * selection only offers it as text, and the compositor has no data-control
 * protocol for the real wl-paste. That's what lets an agent like Claude Code
 * paste a copied screenshot with Ctrl+V.
 */
class GuestClipboard(
    private val context: Context,
) {
    private val manager = context.getSystemService(ClipboardManager::class.java)

    /** Writes [what] ("types", "image" as PNG, or "text") to [reply], a FIFO; nothing if the clipboard has none. */
    fun answer(
        what: String,
        reply: File,
    ) {
        try {
            FileOutputStream(reply).use { out ->
                // Android shows the clipboard only to the focused app: the terminal is, when wl-paste runs from it.
                val item = manager.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0) ?: return
                val image =
                    item.uri?.takeIf {
                        context.contentResolver.getType(it)?.startsWith("image/") == true ||
                            manager.primaryClipDescription?.hasMimeType("image/*") == true
                    }
                when (what) {
                    "types" -> out.write(types(item, image).joinToString("") { "$it\n" }.toByteArray())
                    "image" -> image?.let { png(it, out) }
                    "text" -> item.text?.let { out.write(it.toString().toByteArray()) }
                }
            }
        } catch (e: Exception) {
            // The guest gives up after a few seconds and closes its end: nothing to do about it.
            Log.w(TAG, "answering $what failed", e)
        }
    }

    private fun types(
        item: ClipData.Item,
        image: Uri?,
    ) = buildList {
        if (image != null) add("image/png")
        if (item.text != null) addAll(listOf("text/plain;charset=utf-8", "text/plain"))
    }

    private fun png(
        uri: Uri,
        out: OutputStream,
    ) {
        val bitmap =
            ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, info, _ ->
                // Software, since a hardware bitmap can't be compressed.
                decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                val side = maxOf(info.size.width, info.size.height)
                if (side > MAX_SIDE) {
                    decoder.setTargetSize(info.size.width * MAX_SIDE / side, info.size.height * MAX_SIDE / side)
                }
            }
        try {
            bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
        } finally {
            bitmap.recycle()
        }
    }
}
