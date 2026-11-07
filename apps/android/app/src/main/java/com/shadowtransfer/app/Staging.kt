package com.shadowtransfer.app

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.documentfile.provider.DocumentFile
import java.io.File

object Staging {
    fun copyFiles(context: Context, uris: List<Uri>, batch: File): List<String> {
        val result = mutableListOf<String>()
        for (uri in uris) {
            val name = SafSaver.sanitize(displayName(context, uri) ?: "file")
            val target = uniqueFile(batch, name)
            copy(context, uri, target)
            result.add(target.absolutePath)
        }
        return result
    }

    fun copyFolder(context: Context, treeUri: Uri, batch: File): String? {
        val source = DocumentFile.fromTreeUri(context, treeUri) ?: return null
        val name = SafSaver.sanitize(source.name ?: "Folder")
        val root = uniqueDir(batch, name)
        copyChildren(context, source, root, 0)
        return root.absolutePath
    }

    private fun copyChildren(context: Context, source: DocumentFile, target: File, depth: Int) {
        if (depth > 64) error("Folder nesting is too deep")
        for (child in source.listFiles()) {
            when {
                child.isDirectory -> {
                    val dir = uniqueDir(target, SafSaver.sanitize(child.name ?: "folder"))
                    copyChildren(context, child, dir, depth + 1)
                }
                child.isFile -> {
                    val name = SafSaver.sanitize(child.name ?: "file")
                    copy(context, child.uri, uniqueFile(target, name))
                }
            }
        }
    }

    private fun copy(context: Context, uri: Uri, target: File) {
        context.contentResolver.openInputStream(uri)?.use { input ->
            target.outputStream().use { output -> input.copyTo(output, 64 * 1024) }
        } ?: error("Cannot read ${uri.lastPathSegment}")
    }

    private fun displayName(context: Context, uri: Uri): String? {
        context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (index >= 0) return cursor.getString(index)
                }
            }
        return uri.lastPathSegment
    }

    private fun uniqueFile(parent: File, name: String): File {
        var candidate = File(parent, name)
        if (!candidate.exists()) return candidate
        val dot = name.lastIndexOf('.')
        val stem = if (dot > 0) name.substring(0, dot) else name
        val extension = if (dot > 0) name.substring(dot) else ""
        for (index in 1..9999) {
            candidate = File(parent, "$stem ($index)$extension")
            if (!candidate.exists()) return candidate
        }
        return File(parent, "$stem-${System.currentTimeMillis()}$extension")
    }

    private fun uniqueDir(parent: File, name: String): File {
        var candidate = File(parent, name)
        var index = 1
        while (candidate.exists()) {
            candidate = File(parent, "$name ($index)")
            index += 1
        }
        if (!candidate.mkdirs()) error("Cannot create staging folder")
        return candidate
    }
}
