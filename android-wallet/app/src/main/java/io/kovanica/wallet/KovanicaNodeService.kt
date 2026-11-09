package io.kovanica.wallet

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.BatteryManager
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import org.json.JSONArray
import org.json.JSONObject
import uniffi.kovanica_light.defaultMobileNodeConfig
import uniffi.kovanica_light.verifyPowHeaderChainJson
import uniffi.kovanica_light.verifyPowHeaderJson
import java.util.concurrent.CopyOnWriteArrayList

/**
 * Foreground poller for the Chain 5 pruned-node profile. It does not embed a
 * full node. It loads the mobile profile, waits for unmetered Wi-Fi and
 * charging when that profile asks, and verifies PoW headers on the phone.
 */
class KovanicaNodeService : Service() {
    private var thread: HandlerThread? = null
    private var handler: Handler? = null
    private var chainId: ULong = HYBRID_CHAIN

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopPolling()
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
            NodeRuntime.publish(this, NodeRuntime.snapshot.copy(phase = PHASE_STOPPED, note = "Stopped."))
            return START_NOT_STICKY
        }
        chainId = intent?.getLongExtra(EXTRA_CHAIN_ID, chainId.toLong())?.toULong() ?: chainId
        startInForeground()
        if (intent?.action == ACTION_VERIFY && thread != null) {
            handler?.removeCallbacksAndMessages(null)
            handler?.post { tick() }
            return START_STICKY
        }
        startPolling()
        return START_STICKY
    }

    override fun onDestroy() {
        stopPolling()
        super.onDestroy()
    }

    private fun startInForeground() {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(CHANNEL, "Kovanica node", NotificationManager.IMPORTANCE_LOW)
            manager.createNotificationChannel(channel)
        }
        val notification = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
            .setContentTitle("Kovanica node")
            .setContentText("Checking the Chain 5 tip")
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun startPolling() {
        if (thread != null) return
        val worker = HandlerThread("kovanica-node")
        worker.start()
        thread = worker
        handler = Handler(worker.looper)
        handler?.post { tick() }
    }

    private fun stopPolling() {
        handler?.removeCallbacksAndMessages(null)
        thread?.quitSafely()
        thread = null
        handler = null
    }

    private fun tick() {
        val profile = runCatching { defaultMobileNodeConfig(chainId) }.getOrNull()
        if (profile == null) {
            NodeRuntime.publish(this, NodeRuntime.snapshot.copy(phase = PHASE_STOPPED, note = "The mobile node profile did not load."))
            return
        }
        val base = NodeRuntime.Snapshot(
            phase = PHASE_RUNNING,
            round = "—",
            powHash = "—",
            work = "—",
            cumulative = "—",
            targetBits = "—",
            note = "",
            pruningRounds = profile.pruningWindowRounds.toString(),
            cacheMb = profile.maxRedbCacheMb.toString(),
            bind = profile.localRpcBind,
            ghostdagK = profile.ghostdagK.toString(),
            targetMs = profile.targetBlockMs.toString(),
            headerCount = NodeStore.count(this).toString(),
            storeBytes = NodeStore.databaseBytes(this).toString(),
            pruningBadge = NodeStore.BADGE,
        )
        if (!allowsSync(NodePrefs.wifiAndChargingOnly(this))) {
            NodeRuntime.publish(this, base.copy(phase = PHASE_PAUSED, note = "Paused until the phone is on unmetered Wi-Fi and charging."))
            schedule()
            return
        }
        if (chainId != HYBRID_CHAIN) {
            NodeRuntime.publish(this, base.copy(note = "PoW header checks run on chain 5. This profile is chain $chainId."))
            schedule()
            return
        }
        try {
            val client = NodeClient(NetworkPrefs.rpc(this))
            val dag = client.call("kovanica_getDagInfo", JSONArray())
            var header = NodePow.findHeader(dag)
            if (header == null) {
                val round = (dag as? JSONObject)?.optLong("tipRound", -1L) ?: -1L
                if (round >= 0) {
                    header = NodePow.findHeader(client.call("kovanica_getBlockByRound", JSONArray().put(round)))
                }
            }
            if (header == null) {
                header = NodePow.findHeader(client.call("kovanica_getLeaderHeader", JSONArray()))
            }
            if (header == null) {
                NodeRuntime.publish(this, base.copy(note = "The node did not return a PoW header."))
                schedule()
                return
            }
            val now = System.currentTimeMillis().toULong()
            val parent = NodePow.parentJson(header)
            val verified = verifyPowHeaderJson(HYBRID_CHAIN, header.toString(), parent, now)
            val cumulative = if (parent.isEmpty()) {
                verified.work
            } else {
                runCatching {
                    verifyPowHeaderChainJson(HYBRID_CHAIN, "[$parent,${header}]", now)
                }.getOrDefault(verified.work)
            }
            val remembered = NodeStore.note(this, verified.round.toLong())
            NodeRuntime.publish(
                this,
                base.copy(
                    round = verified.round.toString(),
                    powHash = verified.powHash,
                    work = verified.work,
                    cumulative = cumulative,
                    targetBits = header.opt("target_bits")?.toString() ?: header.opt("targetBits")?.toString() ?: "—",
                    note = "Verified on this phone.",
                    headerCount = remembered.toString(),
                    storeBytes = NodeStore.databaseBytes(this).toString(),
                ),
            )
        } catch (error: Exception) {
            NodeRuntime.publish(this, base.copy(note = error.message ?: "The header check failed."))
        }
        schedule()
    }

    private fun schedule() {
        handler?.postDelayed({ tick() }, POLL_MS)
    }

    private fun allowsSync(enforce: Boolean): Boolean {
        val cm = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val caps = cm.activeNetwork?.let { cm.getNetworkCapabilities(it) }
        val wifi = caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true
        val unmetered = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) == true
        val charging = (getSystemService(Context.BATTERY_SERVICE) as BatteryManager).isCharging
        return NodePow.syncAllowed(enforce, wifi, unmetered, charging)
    }

    companion object {
        const val ACTION_START = "io.kovanica.wallet.NODE_START"
        const val ACTION_STOP = "io.kovanica.wallet.NODE_STOP"
        const val ACTION_VERIFY = "io.kovanica.wallet.NODE_VERIFY"
        const val EXTRA_CHAIN_ID = "chainId"
        private const val CHANNEL = "kovanica-node"
        private const val NOTIFICATION_ID = 7
        private const val POLL_MS = 20_000L
        private val HYBRID_CHAIN = 5uL
    }
}

object NodeRuntime {
    const val PHASE_RUNNING = "Running"
    const val PHASE_PAUSED = "Paused (Battery/Metered)"
    const val PHASE_STOPPED = "Stopped"

    const val ACTION_UPDATED = "io.kovanica.wallet.NODE_UPDATED"

    data class Snapshot(
        val phase: String = PHASE_STOPPED,
        val round: String = "—",
        val powHash: String = "—",
        val work: String = "—",
        val cumulative: String = "—",
        val targetBits: String = "—",
        val note: String = "Stopped.",
        val pruningRounds: String = "14400",
        val cacheMb: String = "64",
        val bind: String = "127.0.0.1:8545",
        val ghostdagK: String = "5",
        val targetMs: String = "4000",
        val headerCount: String = "0",
        val storeBytes: String = "0",
        val pruningBadge: String = "Pruning window: 14,400 rounds (~16h)",
    )

    @Volatile
    var snapshot: Snapshot = Snapshot()
        private set

    private val listeners = CopyOnWriteArrayList<(Snapshot) -> Unit>()

    fun publish(context: Context, next: Snapshot) {
        snapshot = next
        listeners.forEach { listener -> listener(next) }
        context.sendBroadcast(Intent(ACTION_UPDATED).setPackage(context.packageName))
    }

    fun listen(listener: (Snapshot) -> Unit): () -> Unit {
        listeners.add(listener)
        listener(snapshot)
        return { listeners.remove(listener) }
    }
}

/** User override for the profile's Wi-Fi and charging gate. Default is on. */
object NodePrefs {
    private const val KEY = "node_wifi_charging_only"

    fun wifiAndChargingOnly(context: Context): Boolean =
        context.getSharedPreferences(NetworkPrefs.PREFS, Context.MODE_PRIVATE).getBoolean(KEY, true)

    fun setWifiAndChargingOnly(context: Context, enabled: Boolean) {
        context.getSharedPreferences(NetworkPrefs.PREFS, Context.MODE_PRIVATE)
            .edit()
            .putBoolean(KEY, enabled)
            .apply()
    }
}

/** Counts verified tips inside the 14,400-round window and measures any local db files. */
object NodeStore {
    const val BADGE = "Pruning window: 14,400 rounds (~16h)"
    private const val KEY = "verified_header_rounds"

    fun count(context: Context): Int = read(context).size

    fun note(context: Context, round: Long): Int {
        val kept = AirgapReview.retain(read(context), round)
        context.getSharedPreferences(NetworkPrefs.PREFS, Context.MODE_PRIVATE)
            .edit()
            .putString(KEY, kept.sorted().joinToString(","))
            .apply()
        return kept.size
    }

    fun databaseBytes(context: Context): Long {
        val dirs = listOfNotNull(
            context.filesDir,
            context.noBackupFilesDir,
            context.getExternalFilesDir(null),
            context.getDatabasePath("kovanica").parentFile,
        )
        var total = 0L
        val seen = HashSet<String>()
        for (dir in dirs) {
            if (!seen.add(dir.absolutePath) || !dir.isDirectory) continue
            dir.listFiles()?.forEach { file ->
                val name = file.name.lowercase()
                if (file.isFile && (name.endsWith(".db") || name.endsWith(".sqlite") || name.endsWith(".sqlite3") || name.endsWith(".redb"))) {
                    total += file.length()
                }
            }
        }
        return total
    }

    private fun read(context: Context): Set<Long> {
        val raw = context.getSharedPreferences(NetworkPrefs.PREFS, Context.MODE_PRIVATE).getString(KEY, "").orEmpty()
        return raw.split(',').mapNotNull { it.toLongOrNull() }.toSet()
    }
}
