package dev.omalauncher.linux

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.SystemClock
import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import android.system.StructPollfd
import android.util.Log
import java.io.File
import java.io.FileDescriptor
import kotlin.concurrent.thread

private const val TAG = "mic"
const val MIC_RATE = 16000
private const val CHUNK_BYTES = MIC_RATE * 2 / 50 // 20 ms of s16le mono

/** How long PulseAudio may stop reading before the microphone turns off. */
private const val STALL_MS = 500L

/**
 * Records the phone's microphone for the guest: PulseAudio reads a FIFO in the shared runtime dir
 * (module-pipe-source) and we write the microphone into it. The source suspends when nothing in
 * the guest records, and then stops reading: the FIFO fills and we wait in poll() for it to drain,
 * so the microphone is on only while a guest program records.
 */
class MicBridge(
    private val context: Context,
    proot: Proot,
) {
    val fifo = File(proot.runtimeDir, "mic.fifo")

    fun start() {
        thread(name = "mic-bridge", isDaemon = true) { run() }
    }

    private fun run() {
        while (true) {
            val fd =
                try {
                    // Fails with ENXIO until PulseAudio, which keeps it open for reading, has loaded the source.
                    Os.open(fifo.path, OsConstants.O_WRONLY or OsConstants.O_NONBLOCK, 0)
                } catch (_: ErrnoException) {
                    Thread.sleep(1000)
                    continue
                }
            try {
                serve(fd)
            } catch (e: ErrnoException) {
                Log.w(TAG, "mic pipe closed", e) // EPIPE: PulseAudio went away
            } finally {
                Os.close(fd)
            }
            Thread.sleep(1000)
        }
    }

    /** Waits for the guest to record, then records until PulseAudio stops reading. */
    private fun serve(fd: FileDescriptor) {
        val silence = ByteArray(CHUNK_BYTES)
        val poll = StructPollfd().apply { this.fd = fd; events = OsConstants.POLLOUT.toShort() }
        while (true) {
            while (write(fd, silence)) Unit
            Os.poll(arrayOf(poll), -1)
            if (context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
                Log.w(TAG, "the guest records, but the microphone permission isn't granted")
                continue
            }
            record(fd)
        }
    }

    @SuppressLint("MissingPermission") // checked in serve()
    private fun record(fd: FileDescriptor) {
        val minBuffer = AudioRecord.getMinBufferSize(MIC_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val recorder =
            AudioRecord(
                MediaRecorder.AudioSource.VOICE_RECOGNITION,
                MIC_RATE,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
                maxOf(minBuffer, CHUNK_BYTES * 10),
            )
        if (recorder.state != AudioRecord.STATE_INITIALIZED) {
            Log.w(TAG, "no microphone")
            recorder.release()
            Thread.sleep(1000)
            return
        }
        val buf = ByteArray(CHUNK_BYTES)
        try {
            recorder.startRecording()
            var stalledSince = 0L
            while (true) {
                val n = recorder.read(buf, 0, buf.size)
                if (n <= 0) break
                val now = SystemClock.uptimeMillis()
                if (write(fd, buf, n)) {
                    stalledSince = 0L
                } else if (stalledSince == 0L) {
                    stalledSince = now
                } else if (now - stalledSince > STALL_MS) {
                    break
                }
            }
        } finally {
            recorder.stop()
            recorder.release()
        }
    }

    /** Writes without blocking; false when the FIFO is full. Writes this small (under PIPE_BUF) are all or nothing. */
    private fun write(
        fd: FileDescriptor,
        bytes: ByteArray,
        length: Int = bytes.size,
    ): Boolean =
        try {
            Os.write(fd, bytes, 0, length)
            true
        } catch (e: ErrnoException) {
            if (e.errno != OsConstants.EAGAIN) throw e
            false
        }
}
