package com.shadowtransfer.app

import android.content.Context
import android.net.Uri
import android.webkit.MimeTypeMap
import androidx.documentfile.provider.DocumentFile
import java.io.File

object SafSaver {
    fun label(context: Context, uri: Uri): String =
        DocumentFile.fromTreeUri(context, uri)?.name
            ?: uri.lastPathSegment
            ?: "Selected folder"

    fun importFile(
        context: Context,
        treeUri: Uri,
        source: File,
        relativePath: String,
    ): Result<Uri> = runCatching {
        val resolver = context.contentResolver
        var directory = DocumentFile.fromTreeUri(context, treeUri)
            ?: error("Destination folder is no longer accessible")

        val parts = relativePath.split('/', '\\').filter { it.isNotBlank() }
        val fileName = sanitize(parts.lastOrNull() ?: source.name)
        for (part in parts.dropLast(1)) {
            val clean = sanitize(part)
            directory = directory.findFile(clean)?.takeIf { it.isDirectory }
                ?: directory.createDirectory(clean)
                ?: error("Cannot create folder $clean")
        }

        val unique = uniqueName(directory, fileName)
        val extension = unique.substringAfterLast('.', "").lowercase()
        val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension)
            ?: "application/octet-stream"
        val document = directory.createFile(mime, unique)
            ?: error("Cannot create $unique")
        resolver.openOutputStream(document.uri)?.use { output ->
            source.inputStream().use { input -> input.copyTo(output, 64 * 1024) }
        } ?: error("Cannot write $unique")
        document.uri
    }

    private fun uniqueName(directory: DocumentFile, name: String): String {
        if (directory.findFile(name) == null) return name
        val dot = name.lastIndexOf('.')
        val stem = if (dot > 0) name.substring(0, dot) else name
        val extension = if (dot > 0) name.substring(dot) else ""
        for (index in 1..9999) {
            val candidate = "$stem ($index)$extension"
            if (directory.findFile(candidate) == null) return candidate
        }
        return "$stem-${System.currentTimeMillis()}$extension"
    }

    fun sanitize(name: String): String {
        var cleaned = name
            .replace(Regex("[\\\\/:*?\"<>|]"), "_")
            .filter { !it.isISOControl() }
            .trim()
            .trimEnd('.')
            .take(120)
        if (cleaned.isEmpty()) cleaned = "file"
        return cleaned
    }
}
