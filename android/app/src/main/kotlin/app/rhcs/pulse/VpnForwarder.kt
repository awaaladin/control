package app.rhcs.pulse

import android.net.Network
import android.net.VpnService
import android.os.ParcelFileDescriptor
import android.system.Os
import android.system.OsConstants
import android.system.StructPollfd
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.IOException
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.SocketTimeoutException
import java.nio.ByteBuffer
import java.nio.channels.SocketChannel
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock
import kotlin.random.Random

/**
 * User-space forwarder behind the game VPN (IPv4 only, written from scratch).
 *
 * It reads IP packets of the one routed app from the tunnel, and for every UDP flow / TCP
 * connection opens an ordinary socket of its own, protected from the VPN and bound to the chosen
 * [Network], then relays data both ways, building the reply packets itself. A connection keeps the
 * network it was opened on; only NEW flows use a newly chosen network.
 *
 * Deliberately simple TCP: in-order delivery only (the tunnel is local, so packets are not lost on
 * the way), no retransmission, no window scaling, client window respected, 1360-byte segments.
 * Not handled: IPv6 (not routed at all, see the docs), ICMP (ping) and IP fragments (dropped).
 *
 * Kill switch: if the game sends packets and nothing comes back for 6 s, or anything unexpected
 * happens, [onDead] is called and the VPN is stopped so the traffic returns to the normal network.
 */
class VpnForwarder(
    private val vpn: VpnService,
    private val tun: ParcelFileDescriptor,
    private val onDead: (String) -> Unit,
) {
    private val input = FileInputStream(tun.fileDescriptor)
    private val output = FileOutputStream(tun.fileDescriptor)
    private val writeLock = Any()
    private val pool = Executors.newCachedThreadPool { r -> Thread(r, "pulse-vpn").apply { isDaemon = true } }
    private val udp = ConcurrentHashMap<Long, UdpFlow>()
    private val tcp = ConcurrentHashMap<Long, TcpFlow>()
    @Volatile private var running = true
    @Volatile private var pendingSince = 0L
    private var readerThread: Thread? = null

    fun start() {
        val t = Thread({
            try {
                readLoop()
            } catch (e: Throwable) {
                die("Internal error: ${e.javaClass.simpleName}: ${e.message}")
            }
        }, "pulse-vpn-read")
        t.isDaemon = true
        readerThread = t
        t.start()
        val w = Thread({
            try {
                watchLoop()
            } catch (e: Throwable) {
                die("Internal error: ${e.javaClass.simpleName}: ${e.message}")
            }
        }, "pulse-vpn-watch")
        w.isDaemon = true
        w.start()
    }

    /** Stops everything this forwarder started. Idempotent. */
    fun close() {
        running = false
        try {
            for (f in udp.values) f.close()
            for (f in tcp.values) f.closeQuiet()
        } catch (_: Exception) {
        }
        udp.clear()
        tcp.clear()
        pool.shutdownNow()
        val rt = readerThread
        if (rt != null && rt !== Thread.currentThread()) {
            try {
                rt.join(900)
            } catch (_: InterruptedException) {
            }
        }
    }

    private fun die(reason: String) {
        if (!running) return
        running = false
        try {
            onDead(reason)
        } catch (_: Throwable) {
        }
    }

    private fun spawn(block: () -> Unit) {
        try {
            pool.execute {
                try {
                    block()
                } catch (e: Throwable) {
                    if (running) die("Internal error: ${e.javaClass.simpleName}: ${e.message}")
                }
            }
        } catch (_: java.util.concurrent.RejectedExecutionException) {
        }
    }

    private fun noteSend() {
        if (pendingSince == 0L) pendingSince = System.currentTimeMillis()
    }

    private fun noteReply(bytes: Int) {
        pendingSince = 0L
        PulseVpn.rx.addAndGet(bytes.toLong())
    }

    private fun emit(pkt: ByteArray) {
        try {
            synchronized(writeLock) { output.write(pkt) }
        } catch (e: IOException) {
            if (running) die("Could not write to the tunnel: ${e.message}")
        }
    }

    // ------------------------------------------------------------------ main loops

    private fun readLoop() {
        val buf = ByteArray(65535)
        val pfd = StructPollfd()
        pfd.fd = tun.fileDescriptor
        pfd.events = OsConstants.POLLIN.toShort()
        val errMask = OsConstants.POLLERR or OsConstants.POLLHUP or OsConstants.POLLNVAL
        while (running) {
            pfd.revents = 0
            val n = try {
                Os.poll(arrayOf(pfd), 500)
            } catch (e: android.system.ErrnoException) {
                if (e.errno == OsConstants.EINTR) 0 else throw e
            }
            if (!running) return
            if (n <= 0) continue
            val re = pfd.revents.toInt()
            if ((re and OsConstants.POLLIN) == 0 && (re and errMask) != 0) {
                die("The tunnel closed")
                return
            }
            val len = input.read(buf)
            if (len < 0) {
                die("The tunnel closed")
                return
            }
            if (len > 0) handle(buf, len)
        }
    }

    private fun watchLoop() {
        var tick = 0
        while (running) {
            Thread.sleep(1000)
            val now = System.currentTimeMillis()
            val p = pendingSince
            if (p != 0L && now - p >= NO_REPLY_MS) {
                die("The game sent data but nothing came back for 6 seconds, so the VPN was switched off.")
                return
            }
            if (now - PulseVpn.lastBeat > 60_000L) {
                die("PULSE stopped watching the game, so the VPN was switched off.")
                return
            }
            tick++
            for ((k, f) in udp) {
                val idle = if (f.dstPort == 53) 15_000L else 120_000L
                if (now - f.last > idle) {
                    f.close()
                    udp.remove(k, f)
                }
            }
            for (f in tcp.values) {
                if (now - f.lastActive > 300_000L) f.reset()
            }
            PulseVpn.flows.set(udp.size + tcp.size)
        }
    }

    // ------------------------------------------------------------------ packets from the game

    private fun handle(b: ByteArray, len: Int) {
        if (len < 20) return
        if (((b[0].toInt() ushr 4) and 15) != 4) return // IPv6 and anything else is not routed
        val ihl = (b[0].toInt() and 15) * 4
        val total = u16(b, 2)
        if (ihl < 20 || total < ihl || total > len) return
        if ((u16(b, 6) and 0x3fff) != 0) return // fragments are dropped
        val proto = u8(b, 9)
        val dst = i32(b, 16)
        when (proto) {
            17 -> handleUdp(b, ihl, total, dst)
            6 -> handleTcp(b, ihl, total, dst)
        }
    }

    private fun handleUdp(b: ByteArray, ihl: Int, total: Int, dst: Int) {
        if (total < ihl + 8) return
        val sp = u16(b, ihl)
        val dp = u16(b, ihl + 2)
        val plen = minOf(u16(b, ihl + 4), total - ihl) - 8
        if (plen < 0) return
        val key = flowKey(sp, dst, dp)
        var f = udp[key]
        if (f == null) {
            if (udp.size >= MAX_UDP) return
            val net = PulseVpn.network ?: return
            f = try {
                UdpFlow(key, sp, dst, dp, net)
            } catch (e: IOException) {
                noteSend()
                return
            }
            udp[key] = f
            val flow = f
            spawn { flow.readLoop() }
        }
        noteSend()
        PulseVpn.tx.addAndGet(plen.toLong())
        if (!f.send(b, ihl + 8, plen)) {
            f.close()
            udp.remove(key, f)
        }
    }

    private fun handleTcp(b: ByteArray, ihl: Int, total: Int, dst: Int) {
        if (total < ihl + 20) return
        val sp = u16(b, ihl)
        val dp = u16(b, ihl + 2)
        val seq = i32(b, ihl + 4)
        val ack = i32(b, ihl + 8)
        val off = ((u8(b, ihl + 12) ushr 4) * 4)
        val flags = u8(b, ihl + 13)
        val wnd = u16(b, ihl + 14)
        if (off < 20 || ihl + off > total) return
        val pOff = ihl + off
        val pLen = total - pOff
        val key = flowKey(sp, dst, dp)
        val f = tcp[key]
        val syn = (flags and SYN) != 0
        if (syn && (flags and ACK) == 0) {
            if (f != null) {
                if (f.clientIsn == seq) {
                    f.onRetransmittedSyn()
                    return
                }
                f.reset()
            }
            if (tcp.size >= MAX_TCP) {
                rstFor(dst, dp, sp, seq, ack, flags, pLen)
                return
            }
            val net = PulseVpn.network
            if (net == null) {
                rstFor(dst, dp, sp, seq, ack, flags, pLen)
                return
            }
            noteSend()
            val nf = TcpFlow(key, sp, dst, dp, seq, net)
            tcp[key] = nf
            spawn { nf.connect() }
            return
        }
        if (f == null) {
            if ((flags and RST) == 0) rstFor(dst, dp, sp, seq, ack, flags, pLen)
            return
        }
        if (pLen > 0) {
            noteSend()
            PulseVpn.tx.addAndGet(pLen.toLong())
        }
        f.onSegment(flags, seq, ack, wnd, b, pOff, pLen)
    }

    /** RST for a segment that belongs to no flow, so the game's side does not hang. */
    private fun rstFor(srvIp: Int, srvPort: Int, cliPort: Int, seq: Int, ack: Int, flags: Int, len: Int) {
        if ((flags and ACK) != 0) {
            emit(buildTcp(srvIp, CLIENT_IP, srvPort, cliPort, ack, 0, RST, 0, null, 0, 0, false))
        } else {
            val syn = if ((flags and SYN) != 0) 1 else 0
            emit(buildTcp(srvIp, CLIENT_IP, srvPort, cliPort, 0, seq + len + syn, RST or ACK, 0, null, 0, 0, false))
        }
    }

    // ------------------------------------------------------------------ UDP

    private inner class UdpFlow(val key: Long, val srcPort: Int, val dstIp: Int, val dstPort: Int, net: Network) {
        private val addr: InetAddress = ipAddr(dstIp)
        private val sock: DatagramSocket
        @Volatile var last = System.currentTimeMillis()
        @Volatile private var closed = false

        init {
            val s = DatagramSocket()
            try {
                if (!vpn.protect(s)) throw IOException("could not protect the socket")
                net.bindSocket(s)
                s.soTimeout = 1000
            } catch (e: Throwable) {
                try {
                    s.close()
                } catch (_: Exception) {
                }
                if (e is IOException) throw e
                throw IOException(e.message)
            }
            sock = s
        }

        fun send(b: ByteArray, off: Int, len: Int): Boolean {
            if (closed) return false
            last = System.currentTimeMillis()
            return try {
                sock.send(DatagramPacket(b, off, len, addr, dstPort))
                true
            } catch (e: IOException) {
                false
            }
        }

        fun readLoop() {
            val buf = ByteArray(MTU)
            val pk = DatagramPacket(buf, buf.size)
            while (!closed && running) {
                try {
                    pk.length = buf.size
                    sock.receive(pk)
                } catch (e: SocketTimeoutException) {
                    continue
                } catch (e: IOException) {
                    break
                }
                last = System.currentTimeMillis()
                val src = pk.address?.address ?: continue
                if (src.size != 4 || pk.length > MTU - 28) continue
                noteReply(pk.length)
                emit(buildUdp(i32(src, 0), CLIENT_IP, pk.port, srcPort, buf, pk.length))
            }
            close()
            udp.remove(key, this)
        }

        fun close() {
            closed = true
            try {
                sock.close()
            } catch (_: Exception) {
            }
        }
    }

    // ------------------------------------------------------------------ TCP

    private class Chunk(val data: ByteArray?, val end: Int)

    private inner class TcpFlow(
        val key: Long,
        val srcPort: Int,
        val dstIp: Int,
        val dstPort: Int,
        val clientIsn: Int,
        val net: Network,
    ) {
        private val lk = ReentrantLock()
        private val cond = lk.newCondition()
        private val isn = Random.nextInt()
        private var ch: SocketChannel? = null
        private var open = false
        private var closed = false
        private var sendNext = isn
        private var sendUna = isn
        private var clientNext = clientIsn + 1
        private var acked = clientIsn + 1
        private var clientWnd = 65535
        private var finSent = false
        private var clientFinned = false
        private var lastClientPkt = System.currentTimeMillis()
        private val outQ = LinkedBlockingQueue<Chunk>()
        @Volatile var lastActive = System.currentTimeMillis()

        fun connect() {
            var opened: SocketChannel? = null
            val c: SocketChannel = try {
                val x = SocketChannel.open()
                opened = x
                val s = x.socket()
                if (!vpn.protect(s)) throw IOException("could not protect the socket")
                net.bindSocket(s)
                s.tcpNoDelay = true
                s.connect(InetSocketAddress(ipAddr(dstIp), dstPort), 6000)
                x
            } catch (e: Throwable) {
                try {
                    opened?.close()
                } catch (_: Exception) {
                }
                val was = lk.withLock {
                    val w = closed
                    closed = true
                    w
                }
                tcp.remove(key, this)
                if (!was && running) emit(buildTcp(dstIp, CLIENT_IP, dstPort, srcPort, 0, clientIsn + 1, RST or ACK, 0, null, 0, 0, false))
                return
            }
            val ok = lk.withLock {
                if (closed) {
                    false
                } else {
                    ch = c
                    open = true
                    sendNext = isn + 1
                    sendUna = isn + 1
                    true
                }
            }
            if (!ok) {
                try {
                    c.close()
                } catch (_: Exception) {
                }
                return
            }
            sendSynAck()
            spawn { writerLoop() }
            spawn { readerLoop() }
        }

        private fun sendSynAck() {
            emit(buildTcp(dstIp, CLIENT_IP, dstPort, srcPort, isn, clientIsn + 1, SYN or ACK, 65535, null, 0, 0, true))
        }

        fun onRetransmittedSyn() {
            val o = lk.withLock { open && !closed }
            if (o) sendSynAck()
        }

        fun onSegment(flags: Int, seq: Int, ack: Int, wnd: Int, b: ByteArray, off: Int, len: Int) {
            var needAck = false
            lk.withLock {
                lastActive = System.currentTimeMillis()
                lastClientPkt = lastActive
                if ((flags and RST) != 0) {
                    closeLocked()
                    return
                }
                if (!open || closed) return
                if ((flags and ACK) != 0) {
                    if (ack - sendUna > 0 && ack - sendNext <= 0) sendUna = ack
                    clientWnd = wnd
                    cond.signalAll()
                }
                if (len > 0 && !clientFinned) {
                    if (seq == clientNext) {
                        clientNext += len
                        outQ.add(Chunk(b.copyOfRange(off, off + len), clientNext))
                    } else {
                        needAck = true
                    }
                }
                if ((flags and FIN) != 0) {
                    if (!clientFinned && seq + len == clientNext) {
                        clientNext += 1
                        clientFinned = true
                        outQ.add(Chunk(null, clientNext))
                    } else {
                        needAck = true
                    }
                }
                maybeFinishLocked()
            }
            if (needAck) sendCtl(ACK)
        }

        private fun sendCtl(flags: Int) {
            var seq = 0
            var ack = 0
            lk.withLock {
                seq = sendNext
                ack = acked
            }
            emit(buildTcp(dstIp, CLIENT_IP, dstPort, srcPort, seq, ack, flags, 65535, null, 0, 0, false))
        }

        private fun writerLoop() {
            try {
                while (running) {
                    if (lk.withLock { closed }) return
                    val c = outQ.poll(500, TimeUnit.MILLISECONDS) ?: continue
                    val channel = lk.withLock { ch } ?: return
                    if (c.data != null) {
                        val bb = ByteBuffer.wrap(c.data)
                        while (bb.hasRemaining()) channel.write(bb)
                    } else {
                        channel.socket().shutdownOutput()
                    }
                    lastActive = System.currentTimeMillis()
                    lk.withLock { acked = c.end }
                    sendCtl(ACK)
                    lk.withLock { maybeFinishLocked() }
                }
            } catch (e: IOException) {
                reset()
            }
        }

        private fun readerLoop() {
            val channel = lk.withLock { ch } ?: return
            val bb = ByteBuffer.allocate(MSS)
            try {
                while (running) {
                    bb.clear()
                    val n = channel.read(bb)
                    if (n < 0) {
                        sendFin()
                        return
                    }
                    if (n == 0) continue
                    lastActive = System.currentTimeMillis()
                    var seq = 0
                    var stuck = false
                    lk.withLock {
                        while (!closed) {
                            val inflight = sendNext - sendUna
                            if (inflight <= 0 || inflight + n <= minOf(clientWnd, 65535)) break
                            cond.await(200, TimeUnit.MILLISECONDS)
                            if (System.currentTimeMillis() - lastClientPkt > 60_000L) {
                                stuck = true
                                break
                            }
                        }
                        seq = sendNext
                        sendNext += n
                    }
                    if (stuck) {
                        reset()
                        return
                    }
                    if (lk.withLock { closed }) return
                    noteReply(n)
                    val ack = lk.withLock { acked }
                    emit(buildTcp(dstIp, CLIENT_IP, dstPort, srcPort, seq, ack, ACK or PSH, 65535, bb.array(), 0, n, false))
                }
            } catch (e: IOException) {
                reset()
            }
        }

        private fun sendFin() {
            var seq = 0
            lk.withLock {
                seq = sendNext
                sendNext += 1
                finSent = true
            }
            val ack = lk.withLock { acked }
            emit(buildTcp(dstIp, CLIENT_IP, dstPort, srcPort, seq, ack, FIN or ACK, 65535, null, 0, 0, false))
            lk.withLock { maybeFinishLocked() }
        }

        /** Everything both ways is done and acknowledged. */
        private fun maybeFinishLocked() {
            if (finSent && clientFinned && sendUna == sendNext && acked == clientNext) closeLocked()
        }

        /** Abort: tell the game's side with a RST and drop the connection. */
        fun reset() {
            var doSend = false
            var seq = 0
            var ack = 0
            lk.withLock {
                doSend = open && !closed
                seq = sendNext
                ack = acked
                closeLocked()
            }
            if (doSend && running) emit(buildTcp(dstIp, CLIENT_IP, dstPort, srcPort, seq, ack, RST or ACK, 0, null, 0, 0, false))
        }

        private fun closeLocked() {
            closed = true
            try {
                ch?.close()
            } catch (_: Exception) {
            }
            cond.signalAll()
            tcp.remove(key, this)
        }

        fun closeQuiet() {
            lk.withLock { closeLocked() }
        }
    }

    companion object {
        const val CLIENT_ADDR = "10.77.0.2"
        const val CLIENT_IP = (10 shl 24) or (77 shl 16) or 2
        const val MTU = 1500
        const val MSS = 1360
        private const val NO_REPLY_MS = 6_000L
        private const val MAX_UDP = 200
        private const val MAX_TCP = 150
        private const val FIN = 1
        private const val SYN = 2
        private const val RST = 4
        private const val PSH = 8
        private const val ACK = 16

        private fun u8(b: ByteArray, i: Int) = b[i].toInt() and 0xff
        private fun u16(b: ByteArray, i: Int) = (u8(b, i) shl 8) or u8(b, i + 1)
        private fun i32(b: ByteArray, i: Int) = (u8(b, i) shl 24) or (u8(b, i + 1) shl 16) or (u8(b, i + 2) shl 8) or u8(b, i + 3)
        private fun put16(b: ByteArray, i: Int, v: Int) {
            b[i] = (v ushr 8).toByte()
            b[i + 1] = v.toByte()
        }

        private fun put32(b: ByteArray, i: Int, v: Int) {
            b[i] = (v ushr 24).toByte()
            b[i + 1] = (v ushr 16).toByte()
            b[i + 2] = (v ushr 8).toByte()
            b[i + 3] = v.toByte()
        }

        private fun ipAddr(ip: Int): InetAddress =
            InetAddress.getByAddress(byteArrayOf((ip ushr 24).toByte(), (ip ushr 16).toByte(), (ip ushr 8).toByte(), ip.toByte()))

        private fun flowKey(srcPort: Int, dstIp: Int, dstPort: Int): Long =
            srcPort.toLong() or ((dstIp.toLong() and 0xffffffffL) shl 16) or (dstPort.toLong() shl 48)

        /** Internet checksum of [len] bytes at [off], starting from [init]. */
        private fun csum(b: ByteArray, off: Int, len: Int, init: Long): Int {
            var sum = init
            var i = off
            val end = off + len
            while (i + 1 < end) {
                sum += ((b[i].toInt() and 0xff) shl 8) or (b[i + 1].toInt() and 0xff)
                i += 2
            }
            if (i < end) sum += (b[i].toInt() and 0xff) shl 8
            while ((sum ushr 16) != 0L) sum = (sum and 0xffff) + (sum ushr 16)
            return (sum.toInt().inv()) and 0xffff
        }

        private fun ipHeader(p: ByteArray, total: Int, proto: Int, src: Int, dst: Int) {
            p[0] = 0x45
            put16(p, 2, total)
            p[6] = 0x40 // don't fragment
            p[8] = 64
            p[9] = proto.toByte()
            put32(p, 12, src)
            put32(p, 16, dst)
            put16(p, 10, csum(p, 0, 20, 0L))
        }

        fun buildUdp(srcIp: Int, dstIp: Int, srcPort: Int, dstPort: Int, data: ByteArray, len: Int): ByteArray {
            val total = 28 + len
            val p = ByteArray(total)
            ipHeader(p, total, 17, srcIp, dstIp)
            put16(p, 20, srcPort)
            put16(p, 22, dstPort)
            put16(p, 24, 8 + len)
            // UDP checksum 0 = "none", valid for IPv4
            System.arraycopy(data, 0, p, 28, len)
            return p
        }

        fun buildTcp(
            srcIp: Int, dstIp: Int, srcPort: Int, dstPort: Int, seq: Int, ack: Int, flags: Int, wnd: Int,
            data: ByteArray?, off: Int, len: Int, mss: Boolean,
        ): ByteArray {
            val hl = if (mss) 24 else 20
            val total = 20 + hl + len
            val p = ByteArray(total)
            ipHeader(p, total, 6, srcIp, dstIp)
            val t = 20
            put16(p, t, srcPort)
            put16(p, t + 2, dstPort)
            put32(p, t + 4, seq)
            put32(p, t + 8, ack)
            p[t + 12] = ((hl / 4) shl 4).toByte()
            p[t + 13] = flags.toByte()
            put16(p, t + 14, wnd)
            if (mss) {
                p[t + 20] = 2
                p[t + 21] = 4
                put16(p, t + 22, MSS)
            }
            if (len > 0 && data != null) System.arraycopy(data, off, p, t + hl, len)
            val seg = hl + len
            val pseudo = ((srcIp ushr 16) and 0xffff).toLong() + (srcIp and 0xffff) + ((dstIp ushr 16) and 0xffff) + (dstIp and 0xffff) + 6 + seg
            put16(p, t + 16, csum(p, t, seg, pseudo))
            return p
        }
    }
}
