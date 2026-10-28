package dev.lancem.peegel.sync

import android.content.ClipData
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import androidx.core.content.FileProvider
import java.io.ByteArrayOutputStream
import java.io.File

/** Reading clipboard / shared content, and storing images so other apps can paste them. */
object ClipFiles {
    const val PNG_MIME = "image/png"
    const val JPEG_MIME = "image/jpeg"
    const val MAX_IMAGE_BYTES = 20 * 1024 * 1024
    private const val KEEP_FILES = 30

    data class Content(val mime: String, val body: ByteArray)

    fun isImage(mime: String) = mime == PNG_MIME || mime == JPEG_MIME

    private fun dir(context: Context) = File(context.cacheDir, "clips").apply { mkdirs() }

    fun fileFor(context: Context, sha: String, mime: String): File =
        File(dir(context), "$sha.${if (mime == JPEG_MIME) "jpg" else "png"}")

    /** Saves image bytes (named by hash) and prunes old files. */
    fun store(context: Context, sha: String, mime: String, body: ByteArray): File {
        val file = fileFor(context, sha, mime)
        if (!file.exists()) file.writeBytes(body)
        file.setLastModified(System.currentTimeMillis())
        dir(context).listFiles()
            ?.sortedByDescending { it.lastModified() }
            ?.drop(KEEP_FILES)
            ?.forEach { it.delete() }
        return file
    }

    fun uriFor(context: Context, file: File): Uri =
        FileProvider.getUriForFile(context, "${context.packageName}.clips", file)

    fun imageClip(context: Context, file: File): ClipData =
        ClipData.newUri(context.contentResolver, "Peegel image", uriFor(context, file))

    /** Reads the first item of a clip: an image if it points to one, otherwise its text. */
    fun read(context: Context, clip: ClipData?): Content? {
        val item = clip?.takeIf { it.itemCount > 0 }?.getItemAt(0) ?: return null
        item.uri?.let { uri -> readImage(context, uri)?.let { return it } }
        val text = item.coerceToText(context)?.toString()
        return if (text.isNullOrEmpty()) null else Content(Protocol.TEXT_MIME, text.toByteArray())
    }

    /** Reads an image URI as PNG or JPEG, converting other formats (WebP, HEIC…) to PNG. */
    fun readImage(context: Context, uri: Uri): Content? {
        val type = context.contentResolver.getType(uri) ?: return null
        if (!type.startsWith("image/")) return null
        val bytes = runCatching {
            context.contentResolver.openInputStream(uri)?.use { input ->
                val buffer = ByteArrayOutputStream()
                val chunk = ByteArray(64 * 1024)
                while (true) {
                    val n = input.read(chunk)
                    if (n < 0) break
                    buffer.write(chunk, 0, n)
                    if (buffer.size() > MAX_IMAGE_BYTES) return null
                }
                buffer.toByteArray()
            }
        }.getOrNull() ?: return null
        if (type == PNG_MIME || type == JPEG_MIME) return Content(type, bytes)
        val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: return null
        val png = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        return if (png.size <= MAX_IMAGE_BYTES) Content(PNG_MIME, png) else null
    }

    /** Decodes a small preview for the history list. */
    fun thumbnail(path: String, maxSize: Int = 320): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(path, bounds)
        if (bounds.outWidth <= 0) return null
        var sample = 1
        while (bounds.outWidth / (sample * 2) >= maxSize && bounds.outHeight / (sample * 2) >= maxSize) sample *= 2
        return BitmapFactory.decodeFile(path, BitmapFactory.Options().apply { inSampleSize = sample })
    }
}
