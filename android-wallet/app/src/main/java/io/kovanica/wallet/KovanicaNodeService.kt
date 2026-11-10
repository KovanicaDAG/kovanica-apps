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
import java.io.File
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
            NodeRuntime.publish(this, NodeRuntime.snapshot.copy(phase = NodeRuntime.PHASE_STOPPED, note = "Stopped."))
            return START_NOT_STICKY
        }
        chainId = if (intent?.hasExtra(EXTRA_CHAIN_ID) == true) {
            intent.getLongExtra(EXTRA_CHAIN_ID, chainId.toLong()).toULong()
        } else {
            NetworkPrefs.chainId(this).toULong()
        }
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
            NodeRuntime.publish(this, NodeRuntime.snapshot.copy(phase = NodeRuntime.PHASE_STOPPED, note = "The mobile node profile did not load."))
            return
        }
        val base = NodeRuntime.Snapshot(
            phase = NodeRuntime.PHASE_RUNNING,
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
            peerLine = "peers: — / ${profile.maxPeers} max",
            intervalLine = if (chainId == HYBRID_CHAIN) {
                AirgapReview.intervalLabel(NodeStore.recentTimestamps(this), profile.targetBlockMs.toLong())
            } else {
                "Recent block interval: shown on chain 5"
            },
            intervalTone = if (chainId == HYBRID_CHAIN) {
                AirgapReview.intervalTone(NodeStore.recentTimestamps(this), profile.targetBlockMs.toLong())
            } else {
                "idle"
            },
        )
        if (!allowsSync(NodePrefs.wifiAndChargingOnly(this))) {
            NodeRuntime.publish(this, base.copy(phase = NodeRuntime.PHASE_PAUSED, note = "Paused until the phone is on unmetered Wi-Fi and charging."))
            schedule()
            return
        }
        val rpcUrl = NetworkPrefs.rpc(this)
        val client = NodeClient(rpcUrl)
        val withPeers = base.copy(peerLine = "peers: ${readPeerCount(client)} / ${profile.maxPeers} max")
        if (chainId != HYBRID_CHAIN) {
            NodeRuntime.publish(this, withPeers.copy(note = "PoW header checks run on chain 5. This profile is chain $chainId."))
            schedule()
            return
        }
        val p2pResult = runCatching {
            uniffi.kovanica_light.syncMobileNodePeer(NodeStore.localDbPath(this), rpcUrl, chainId, 32uL)
        }.getOrNull()
        try {
            val dag = client.call("kovanica_getDagInfo", JSONArray())
            var header = NodePow.findHeader(dag)
            if (header == null) {
                val round = (dag as? JSONObject)?.optLong("tipRound", -1L) ?: -1L
                if (round >= 0) {
                    header = NodePow.findHeader(client.call("kovanica_getBlockHeaderByNumber", JSONArray().put(round)))
                        ?: NodePow.findHeader(client.call("kovanica_getBlockByRound", JSONArray().put(round)))
                }
            }
            if (header == null) {
                header = NodePow.findHeader(client.call("kovanica_getLeaderHeader", JSONArray()))
            }
            if (header == null) {
                val noteMsg = if (p2pResult != null && p2pResult.syncedBlocks > 0uL) {
                    "P2P synced ${p2pResult.syncedBlocks} blocks (tip: ${p2pResult.newTipRound})."
                } else {
                    "The node did not return a PoW header."
                }
                NodeRuntime.publish(this, withPeers.copy(note = noteMsg))
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
            val parentRound = parentRound(parent)
            if (parentRound != null) NodeStore.save(this, parentRound, parent)
            val remembered = NodeStore.save(this, verified.round.toLong(), header.toString())
            val interval = AirgapReview.intervalLabel(NodeStore.recentTimestamps(this), profile.targetBlockMs.toLong())
            val tone = AirgapReview.intervalTone(NodeStore.recentTimestamps(this), profile.targetBlockMs.toLong())
            val syncNote = if (p2pResult != null && p2pResult.syncedBlocks > 0uL) {
                "P2P synced ${p2pResult.syncedBlocks} blocks (tip: ${p2pResult.newTipRound}). Verified on phone."
            } else {
                "Verified on this phone."
            }
            NodeRuntime.publish(
                this,
                withPeers.copy(
                    round = verified.round.toString(),
                    powHash = verified.powHash,
                    work = verified.work,
                    cumulative = cumulative,
                    targetBits = header.opt("target_bits")?.toString() ?: header.opt("targetBits")?.toString() ?: "—",
                    note = syncNote,
                    headerCount = remembered.toString(),
                    storeBytes = NodeStore.databaseBytes(this).toString(),
                    intervalLine = interval,
                    intervalTone = tone,
                ),
            )
        } catch (error: Exception) {
            NodeRuntime.publish(this, withPeers.copy(note = error.message ?: "The header check failed."))
        }
        schedule()
    }

    private fun readPeerCount(client: NodeClient): String {
        val sync = runCatching { client.call("kovanica_syncStatus", JSONArray()) }.getOrNull()
        AirgapReview.peerCount(sync)?.let { return it }
        val peers = runCatching { client.call("kovanica_getPeers", JSONArray()) }.getOrNull()
        return AirgapReview.peerCount(peers) ?: "—"
    }

    private fun parentRound(parent: String): Long? {
        if (parent.isBlank()) return null
        val round = runCatching { JSONObject(parent).optLong("round", -1L) }.getOrDefault(-1L)
        return round.takeIf { it >= 0L }
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
        val peerLine: String = "peers: — / 12 max",
        val intervalLine: String = "Recent block interval: —",
        val intervalTone: String = "idle",
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

/** Verified headers on disk under filesDir/headers, pruned to the last 14,400 rounds. */
object NodeStore {
    const val BADGE = "Pruning window: 14,400 rounds (~16h)"

    fun localDbPath(context: Context): String {
        return File(context.filesDir, "mobile_node.redb").absolutePath
    }

    fun count(context: Context): Int {
        val stats = runCatching {
            uniffi.kovanica_light.getLocalStorageStats(localDbPath(context))
        }.getOrNull()
        if (stats != null && stats.totalBlocks > 0uL) {
            return stats.totalBlocks.toInt()
        }
        val dir = File(context.filesDir, "headers")
        if (!dir.isDirectory) return 0
        return dir.listFiles()?.count { roundOf(it) != null } ?: 0
    }

    fun tipRound(context: Context): Long {
        val stats = runCatching {
            uniffi.kovanica_light.getLocalStorageStats(localDbPath(context))
        }.getOrNull()
        return stats?.tipRound?.toLong() ?: 0L
    }

    fun save(context: Context, round: Long, headerJson: String): Int {
        val dir = File(context.filesDir, "headers").apply { mkdirs() }
        write(dir, round, headerJson)
        val files = dir.listFiles().orEmpty()
        val kept = AirgapReview.retain(files.mapNotNull { roundOf(it) }.toSet(), round)
        files.forEach { file ->
            val stored = roundOf(file) ?: return@forEach
            if (stored !in kept) file.delete()
        }
        // Ingest into embedded RedB mobile node database
        runCatching {
            uniffi.kovanica_light.storeVerifiedBlockHeader(localDbPath(context), headerJson)
        }
        pruneEmbeddedDatabases(context, round)
        return kept.size
    }

    fun pruneEmbeddedDatabases(context: Context, currentRound: Long): Long {
        if (currentRound <= 14_400L) return 0L
        val cutoff = (currentRound - 14_400L).toULong()
        var totalPruned = 0L
        val candidateDirs = listOfNotNull(
            context.filesDir,
            context.noBackupFilesDir,
            context.getDatabasePath("kovanica").parentFile,
        )
        val seen = HashSet<String>()
        for (dir in candidateDirs) {
            if (!seen.add(dir.absolutePath) || !dir.isDirectory) continue
            dir.listFiles()?.forEach { file ->
                if (file.isFile && file.name.lowercase().endsWith(".redb")) {
                    runCatching {
                        val summary = uniffi.kovanica_light.pruneLocalNodeStorage(file.absolutePath, cutoff)
                        if (summary.pruned) {
                            totalPruned += (summary.prunedBlocks + summary.prunedDagBlocks).toLong()
                        }
                    }
                }
            }
        }
        return totalPruned
    }

    fun recentTimestamps(context: Context, limit: Int = 32): List<Long> {
        val dir = File(context.filesDir, "headers")
        if (!dir.isDirectory) return emptyList()
        return dir.listFiles().orEmpty()
            .mapNotNull { file -> roundOf(file)?.let { round -> round to file } }
            .sortedBy { it.first }
            .takeLast(limit)
            .mapNotNull { (_, file) ->
                val obj = runCatching { JSONObject(file.readText()) }.getOrNull() ?: return@mapNotNull null
                val stamp = when {
                    obj.has("timestamp") -> obj.optLong("timestamp", -1L)
                    obj.has("timestamp_ms") -> obj.optLong("timestamp_ms", -1L)
                    else -> -1L
                }
                stamp.takeIf { it > 0L }
            }
    }

    fun databaseBytes(context: Context): Long {
        var total = treeBytes(File(context.filesDir, "headers"))
        val dirs = listOfNotNull(
            context.filesDir,
            context.noBackupFilesDir,
            context.getExternalFilesDir(null),
            context.getDatabasePath("kovanica").parentFile,
        )
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

    private fun write(dir: File, round: Long, json: String) {
        val target = File(dir, "$round.json")
        val tmp = File(dir, ".$round.json.tmp")
        tmp.writeText(json)
        if (target.exists() && !target.delete()) tmp.delete()
        if (!tmp.renameTo(target)) {
            target.writeText(json)
            tmp.delete()
        }
    }

    private fun roundOf(file: File): Long? {
        if (!file.isFile || !file.name.endsWith(".json") || file.name.startsWith(".")) return null
        return file.name.removeSuffix(".json").toLongOrNull()
    }

    private fun treeBytes(dir: File): Long {
        if (!dir.isDirectory) return 0L
        var total = 0L
        dir.listFiles()?.forEach { file ->
            total += if (file.isDirectory) treeBytes(file) else file.length()
        }
        return total
    }
}
