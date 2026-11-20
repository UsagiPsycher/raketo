package app.raketo

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothSocket
import android.bluetooth.BluetoothStatusCodes
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.Intent
import android.os.Build
import android.provider.OpenableColumns
import android.util.Base64
import androidx.activity.result.ActivityResult
import app.tauri.PermissionState
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.IOException
import java.io.OutputStream
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

private fun u16(v: Int): UUID = UUID.fromString(String.format("0000%04x-0000-1000-8000-00805f9b34fb", v))

/** Write characteristics tried in order, as in crates/bt/src/ble.rs. */
private val WRITE = listOf(u16(0xAE01), u16(0xFF02), u16(0xAB01), UUID.fromString("49535343-8841-43f4-a8d4-ecbe34729bb3"), u16(0xAE81))
private val NOTIFY = listOf(u16(0xAE02), u16(0xAE04), u16(0xAB03), u16(0xFF03), u16(0xAE82))
private val CCCD = u16(0x2902)
private val SPP = u16(0x1101)

/**
 * Bluetooth for the Rust side: scan, connect over BLE GATT or classic SPP, write.
 * Commands run on the calling Rust worker thread, so they may block.
 */
@TauriPlugin(
  permissions = [
    Permission(strings = [Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT], alias = "bluetooth"),
    Permission(strings = [Manifest.permission.ACCESS_FINE_LOCATION], alias = "location"),
  ]
)
@SuppressLint("MissingPermission")
class RaketoPlugin(private val activity: Activity) : Plugin(activity) {
  private val adapter: BluetoothAdapter?
    get() = (activity.getSystemService(Context.BLUETOOTH_SERVICE) as BluetoothManager).adapter
  private val alias get() = if (Build.VERSION.SDK_INT >= 31) "bluetooth" else "location"
  private fun allowed() = getPermissionState(alias) == PermissionState.GRANTED

  // Classic connection
  @Volatile private var socket: BluetoothSocket? = null
  @Volatile private var out: OutputStream? = null

  // BLE connection
  @Volatile private var gatt: BluetoothGatt? = null
  @Volatile private var writeChar: BluetoothGattCharacteristic? = null
  @Volatile private var mtu = 23
  @Volatile private var step: CountDownLatch? = null
  @Volatile private var stepOk = false

  /** Starts an asynchronous GATT operation and waits for its callback. */
  private fun await(ms: Long, start: () -> Boolean): Boolean {
    val l = CountDownLatch(1)
    step = l
    stepOk = false
    if (!start()) return false
    return l.await(ms, TimeUnit.MILLISECONDS) && stepOk
  }

  private fun done(ok: Boolean) {
    stepOk = ok
    step?.countDown()
  }

  private val callback = object : BluetoothGattCallback() {
    override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) {
      if (newState == BluetoothProfile.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS) {
        done(true)
      } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
        done(false)
        if (g == gatt) Native.onClosed()
      }
    }

    override fun onServicesDiscovered(g: BluetoothGatt, status: Int) = done(status == BluetoothGatt.GATT_SUCCESS)

    override fun onMtuChanged(g: BluetoothGatt, m: Int, status: Int) {
      if (status == BluetoothGatt.GATT_SUCCESS) mtu = m
      done(true)
    }

    override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) = done(status == BluetoothGatt.GATT_SUCCESS)

    override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) = done(status == BluetoothGatt.GATT_SUCCESS)

    override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) = Native.onData(value)

    @Deprecated("Android 12 and older")
    override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
      if (Build.VERSION.SDK_INT < 33) c.value?.let { Native.onData(it.copyOf()) }
    }
  }

  // Scanning

  @Command
  fun scan(invoke: Invoke) {
    if (!allowed()) {
      activity.runOnUiThread { requestPermissionForAlias(alias, invoke, "scanAllowed") }
      return
    }
    val a = adapter ?: return invoke.reject("no Bluetooth adapter")
    if (!a.isEnabled) {
      activity.runOnUiThread { startActivityForResult(invoke, Intent(BluetoothAdapter.ACTION_REQUEST_ENABLE), "bluetoothEnabled") }
      return
    }
    doScan(invoke)
  }

  @PermissionCallback
  fun scanAllowed(invoke: Invoke) {
    if (!allowed()) return invoke.reject("Bluetooth permission denied")
    Thread { scan(invoke) }.start()
  }

  @ActivityCallback
  fun bluetoothEnabled(invoke: Invoke, result: ActivityResult) {
    if (adapter?.isEnabled != true) return invoke.reject("Bluetooth is off")
    Thread { doScan(invoke) }.start()
  }

  private fun doScan(invoke: Invoke) {
    val a = adapter ?: return invoke.reject("no Bluetooth adapter")
    val seconds = invoke.getArgs().getInteger("seconds", 6)
    val seen = ConcurrentHashMap<String, JSObject>()
    val cb = object : ScanCallback() {
      override fun onScanResult(type: Int, r: ScanResult) {
        val name = r.scanRecord?.deviceName ?: try { r.device.name } catch (e: SecurityException) { null } ?: return
        seen[r.device.address] = JSObject().apply {
          put("name", name)
          put("address", r.device.address)
          put("rssi", r.rssi)
        }
      }
    }
    val scanner = a.bluetoothLeScanner ?: return invoke.reject("Bluetooth is off")
    try {
      scanner.startScan(null, ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build(), cb)
      Thread.sleep(seconds * 1000L)
    } catch (e: Exception) {
      return invoke.reject(e.message ?: "scan failed")
    } finally {
      try { scanner.stopScan(cb) } catch (_: Exception) {}
    }
    val list = JSArray()
    seen.values.forEach { list.put(it) }
    invoke.resolve(JSObject().apply { put("devices", list) })
  }

  // Connection

  @Command
  fun open(invoke: Invoke) {
    val args = invoke.getArgs()
    val address = args.getString("address") ?: return invoke.reject("no address")
    val spp = args.getBoolean("spp", false)
    val wanted = args.getInteger("mtu", 23)
    closeAll()
    val a = adapter ?: return invoke.reject("no Bluetooth adapter")
    if (!allowed()) return invoke.reject("Bluetooth permission denied")
    val dev = try { a.getRemoteDevice(address) } catch (e: IllegalArgumentException) { return invoke.reject("bad address") }
    try {
      val chunk = if (spp) openSpp(a, dev) else openGatt(dev, wanted)
      invoke.resolve(JSObject().apply { put("chunk", chunk) })
    } catch (e: Exception) {
      closeAll()
      invoke.reject(e.message ?: e.toString())
    }
  }

  private fun openSpp(a: BluetoothAdapter, dev: BluetoothDevice): Int {
    a.cancelDiscovery()
    var last: Exception? = null
    val attempts: List<() -> BluetoothSocket> = listOf(
      { dev.createInsecureRfcommSocketToServiceRecord(SPP) },
      { dev.createRfcommSocketToServiceRecord(SPP) },
      // Some printers only answer on channel 1 without an SDP lookup.
      { dev.javaClass.getMethod("createRfcommSocket", Int::class.javaPrimitiveType).invoke(dev, 1) as BluetoothSocket },
    )
    for (make in attempts) {
      var s: BluetoothSocket? = null
      try {
        s = make()
        s.connect()
        socket = s
        out = s.outputStream
        val input = s.inputStream
        Thread {
          val buf = ByteArray(1024)
          try {
            while (true) {
              val n = input.read(buf)
              if (n < 0) break
              if (n > 0) Native.onData(buf.copyOf(n))
            }
          } catch (_: IOException) {
          }
          if (socket === s) Native.onClosed()
        }.start()
        return 0
      } catch (e: Exception) {
        last = e
        try { s?.close() } catch (_: IOException) {}
      }
    }
    throw IOException("classic Bluetooth connection failed: ${last?.message}")
  }

  private fun openGatt(dev: BluetoothDevice, wanted: Int): Int {
    var g: BluetoothGatt? = null
    for (attempt in 0 until 3) {
      var made: BluetoothGatt? = null
      val ok = await(12000) {
        made = dev.connectGatt(activity, false, callback, BluetoothDevice.TRANSPORT_LE)
        gatt = made
        made != null
      }
      if (ok) {
        g = made
        break
      }
      made?.close()
      gatt = null
      Thread.sleep(400)
    }
    g ?: throw IOException("could not connect to the printer")
    if (!await(10000) { g.discoverServices() }) throw IOException("services not found")
    mtu = 23
    if (wanted > 23) await(3000) { g.requestMtu(wanted) }
    val chars = g.services.flatMap { it.characteristics }
    val w = WRITE.firstNotNullOfOrNull { u -> chars.firstOrNull { it.uuid == u } }
      ?: chars.firstOrNull { it.properties and BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE != 0 }
      ?: throw IOException("no write characteristic")
    writeChar = w
    for (c in chars) {
      val notifies = c.properties and BluetoothGattCharacteristic.PROPERTY_NOTIFY != 0
      if (!notifies || !(c.uuid in NOTIFY || c.service.uuid == w.service.uuid)) continue
      g.setCharacteristicNotification(c, true)
      val d = c.getDescriptor(CCCD) ?: continue
      await(3000) {
        if (Build.VERSION.SDK_INT >= 33) {
          g.writeDescriptor(d, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE) == BluetoothStatusCodes.SUCCESS
        } else {
          @Suppress("DEPRECATION")
          d.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
          @Suppress("DEPRECATION")
          g.writeDescriptor(d)
        }
      }
    }
    return minOf(mtu, maxOf(wanted, 23)) - 3
  }

  @Command
  fun write(invoke: Invoke) {
    val data = Base64.decode(invoke.getArgs().getString("data") ?: "", Base64.NO_WRAP)
    try {
      out?.let {
        it.write(data)
        it.flush()
        return invoke.resolve()
      }
      val g = gatt ?: return invoke.reject("not connected")
      val c = writeChar ?: return invoke.reject("not connected")
      if (gattWrite(g, c, data)) return invoke.resolve()
      // The link may not take packets this large: fall back to 20 bytes.
      if (data.size > 20 && data.toList().chunked(20).all { gattWrite(g, c, it.toByteArray()) }) return invoke.resolve()
      invoke.reject("write failed")
    } catch (e: Exception) {
      invoke.reject(e.message ?: "write failed")
    }
  }

  private fun gattWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, data: ByteArray): Boolean {
    for (attempt in 0 until 3) {
      val ok = await(3000) {
        if (Build.VERSION.SDK_INT >= 33) {
          g.writeCharacteristic(c, data, BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE) == BluetoothStatusCodes.SUCCESS
        } else {
          @Suppress("DEPRECATION")
          c.writeType = BluetoothGattCharacteristic.WRITE_TYPE_NO_RESPONSE
          @Suppress("DEPRECATION")
          c.value = data
          @Suppress("DEPRECATION")
          g.writeCharacteristic(c)
        }
      }
      if (ok) return true
      Thread.sleep(15)
    }
    return false
  }

  @Command
  fun close(invoke: Invoke) {
    closeAll()
    invoke.resolve()
  }

  private fun closeAll() {
    val s = socket
    socket = null
    out = null
    try { s?.close() } catch (_: IOException) {}
    val g = gatt
    gatt = null
    writeChar = null
    try {
      g?.disconnect()
      g?.close()
    } catch (_: Exception) {}
  }

  // Saving files through the system document picker

  @Volatile private var pendingSave: String? = null

  @Command
  fun saveDocument(invoke: Invoke) {
    val args = invoke.getArgs()
    pendingSave = args.getString("contents") ?: ""
    val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
      addCategory(Intent.CATEGORY_OPENABLE)
      type = "application/octet-stream"
      putExtra(Intent.EXTRA_TITLE, args.getString("name") ?: "layout.rkt")
    }
    activity.runOnUiThread { startActivityForResult(invoke, intent, "documentSaved") }
  }

  @ActivityCallback
  fun documentSaved(invoke: Invoke, result: ActivityResult) {
    val uri = result.data?.data
    val contents = pendingSave
    pendingSave = null
    if (result.resultCode != Activity.RESULT_OK || uri == null || contents == null) {
      return invoke.resolve(JSObject().apply { put("name", org.json.JSONObject.NULL) })
    }
    Thread {
      try {
        activity.contentResolver.openOutputStream(uri, "wt")?.use { it.write(contents.toByteArray()) }
        var name: String? = null
        activity.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
          if (it.moveToFirst()) name = it.getString(0)
        }
        invoke.resolve(JSObject().apply { put("name", name ?: "layout.rkt") })
      } catch (e: Exception) {
        invoke.reject(e.message ?: "could not save")
      }
    }.start()
  }
}
