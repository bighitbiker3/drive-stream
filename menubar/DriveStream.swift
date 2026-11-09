// Drive Stream menu bar app: the face of the `drivestream` service. Shows whether the drive
// is mounted and signed in, live upload progress and recent uploads, and
// opens the drive, the dashboard and Google sign-in. Everything comes from
// the daemon's local API at http://127.0.0.1:7317.

import AppKit
import ServiceManagement
import SwiftUI

let api = URL(string: ProcessInfo.processInfo.environment["DRIVESTREAM_API"] ?? "http://127.0.0.1:7317")!

struct Upload: Identifiable {
    let id: String
    let name: String
    let folder: String
    let bytes: Double
    let sent: Double
    let state: String
    let detail: String
}

struct Done: Identifiable {
    let id: String
    let name: String
    let bytes: Double
    let finishedAt: Double
    let verified: Bool?
    let failed: Bool
}

func fmtBytes(_ n: Double) -> String {
    if n >= 1e9 { return String(format: n >= 1e10 ? "%.1f GB" : "%.2f GB", n / 1e9) }
    if n >= 1e6 { return String(format: n >= 1e8 ? "%.0f MB" : "%.1f MB", n / 1e6) }
    if n >= 1e3 { return String(format: "%.0f KB", n / 1e3) }
    return String(format: "%.0f B", n)
}

func fmtDuration(_ s: Double) -> String {
    if s < 60 { return "\(max(1, Int(s.rounded())))s" }
    if s < 3600 { return "\(Int((s / 60).rounded())) min" }
    return String(format: "%.1f h", s / 3600)
}

@MainActor
final class Model: ObservableObject {
    @Published var reachable = false
    @Published var phase = "Connecting…"
    @Published var mounted = false
    @Published var email: String?
    @Published var signedIn = false
    @Published var problem: String?
    @Published var mountpoint = NSHomeDirectory() + "/GoogleDriveStream"
    @Published var uploads: [Upload] = []
    @Published var recent: [Done] = []
    @Published var cache = ""
    @Published var openAtLogin = SMAppService.mainApp.status == .enabled
    private var speeds: [String: (t: Double, sent: Double, v: Double)] = [:]
    private var timer: Timer?

    var needsSignIn: Bool { reachable && (!signedIn || problem != nil) }
    var uploading: Bool { uploads.contains { $0.state == "uploading" } }
    var bytesLeft: Double { uploads.reduce(0) { $0 + max(0, $1.bytes - $1.sent) } }

    var icon: String {
        if !reachable { return "externaldrive.badge.xmark" }
        if needsSignIn { return "exclamationmark.icloud" }
        if !uploads.isEmpty { return "icloud.and.arrow.up" }
        return "externaldrive.badge.icloud"
    }

    func start() {
        guard timer == nil else { return }
        Task { await poll() }
        timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            Task { @MainActor in await self?.poll() }
        }
    }

    func speed(of u: Upload) -> Double { speeds[u.id]?.v ?? 0 }

    func poll() async {
        var req = URLRequest(url: api.appendingPathComponent("api/status"))
        req.timeoutInterval = 3
        guard let (data, resp) = try? await URLSession.shared.data(for: req),
              (resp as? HTTPURLResponse)?.statusCode == 200,
              let st = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            reachable = false
            phase = "Background service not running"
            return
        }
        reachable = true
        phase = st["phase"] as? String ?? ""
        mounted = st["mounted"] as? Bool ?? false
        if let m = st["mountpoint"] as? String { mountpoint = m }
        let acct = st["account"] as? [String: Any] ?? [:]
        email = acct["email"] as? String
        signedIn = acct["signed_in"] as? Bool ?? false
        problem = acct["problem"] as? String
        let up = st["uploads"] as? [String: Any]
        let now = Date().timeIntervalSince1970
        uploads = (up?["active"] as? [[String: Any]] ?? []).map { a in
            let path = a["path"] as? String ?? ""
            let parts = path.split(separator: "/").map(String.init)
            let u = Upload(id: path, name: parts.last ?? path, folder: parts.dropLast().joined(separator: "/"),
                           bytes: a["bytes"] as? Double ?? 0, sent: a["sent"] as? Double ?? 0,
                           state: a["state"] as? String ?? "", detail: a["detail"] as? String ?? "")
            if u.state == "uploading" {
                var v = speeds[u.id]?.v ?? 0
                if let prev = speeds[u.id], now > prev.t, u.sent >= prev.sent {
                    v = 0.6 * v + 0.4 * ((u.sent - prev.sent) / (now - prev.t))
                }
                speeds[u.id] = (now, u.sent, v)
            }
            return u
        }
        recent = (up?["history"] as? [[String: Any]] ?? []).prefix(3).map { h in
            let path = h["path"] as? String ?? ""
            return Done(id: path + "\(h["finished_at"] ?? 0)", name: path.split(separator: "/").last.map(String.init) ?? path,
                        bytes: h["bytes"] as? Double ?? 0, finishedAt: h["finished_at"] as? Double ?? 0,
                        verified: h["verified"] as? Bool, failed: !(h["error"] is NSNull) && h["error"] != nil)
        }
        if let d = st["drive"] as? [String: Any], let used = d["cache_bytes"] as? Double, let cap = d["cache_budget"] as? Double {
            cache = "Cache \(fmtBytes(used)) of \(fmtBytes(cap))"
        }
        openAtLogin = SMAppService.mainApp.status == .enabled
    }

    func openDrive() { NSWorkspace.shared.open(URL(fileURLWithPath: mountpoint)) }
    func openDashboard(_ anchor: String = "") { NSWorkspace.shared.open(URL(string: api.absoluteString + "/" + anchor)!) }

    func toggleOpenAtLogin() {
        do {
            if SMAppService.mainApp.status == .enabled { try SMAppService.mainApp.unregister() } else { try SMAppService.mainApp.register() }
        } catch {
            NSLog("Open at Login: \(error)")
        }
        openAtLogin = SMAppService.mainApp.status == .enabled
    }
}

struct Bar: View {
    let fraction: Double
    let active: Bool
    var body: some View {
        GeometryReader { g in
            ZStack(alignment: .leading) {
                Capsule().fill(Color.primary.opacity(0.1))
                Capsule().fill(active ? Color.accentColor : Color.secondary.opacity(0.5))
                    .frame(width: max(4, g.size.width * min(1, max(0, fraction))))
            }
        }
        .frame(height: 5)
    }
}

struct PillButton: View {
    let title: String
    let symbol: String
    var prominent = false
    let action: () -> Void
    var body: some View {
        Button(action: action) {
            Label(title, systemImage: symbol)
                .font(.system(size: 12, weight: .semibold))
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
                .background(prominent ? Color.accentColor : Color.primary.opacity(0.08), in: RoundedRectangle(cornerRadius: 7))
                .foregroundStyle(prominent ? Color.white : Color.primary)
        }
        .buttonStyle(.plain)
    }
}

struct Panel: View {
    @EnvironmentObject var m: Model

    var statusColor: Color {
        if !m.reachable || m.problem != nil { return .red }
        return m.mounted ? .green : .orange
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 10) {
                if let icon = NSImage(named: "AppIcon") {
                    Image(nsImage: icon).resizable().frame(width: 34, height: 34)
                } else {
                    Image(systemName: "externaldrive.fill.badge.icloud")
                        .font(.system(size: 22)).foregroundStyle(Color.accentColor)
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text("Drive Stream").font(.system(size: 13, weight: .bold))
                    HStack(spacing: 5) {
                        Circle().fill(statusColor).frame(width: 7, height: 7)
                        Text(m.problem != nil ? "Sign-in expired" : m.phase).font(.system(size: 11)).foregroundStyle(.secondary).lineLimit(2)
                    }
                    if let e = m.email { Text(e).font(.system(size: 11)).foregroundStyle(.secondary) }
                }
                Spacer()
            }

            if m.needsSignIn {
                VStack(alignment: .leading, spacing: 8) {
                    Text(m.problem ?? "Sign in to Google so your drive can mount.")
                        .font(.system(size: 12)).fixedSize(horizontal: false, vertical: true)
                    PillButton(title: "Sign in to Google…", symbol: "person.badge.key", prominent: true) { m.openDashboard("#signin") }
                }
                .padding(10)
                .background(Color.red.opacity(0.1), in: RoundedRectangle(cornerRadius: 9))
            } else if !m.reachable {
                Text("Drive Stream's background service isn't responding. It starts at login; if it doesn't, run install.sh again.")
                    .font(.system(size: 12)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }

            if m.reachable && !m.needsSignIn {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text("Uploads").font(.system(size: 12, weight: .semibold))
                        Spacer()
                        if !m.uploads.isEmpty {
                            Text("\(m.uploads.count) left · \(fmtBytes(m.bytesLeft))").font(.system(size: 11)).foregroundStyle(.secondary)
                        }
                    }
                    if m.uploads.isEmpty {
                        Text("All caught up.").font(.system(size: 12)).foregroundStyle(.secondary)
                    }
                    ForEach(m.uploads.prefix(4)) { u in
                        VStack(alignment: .leading, spacing: 3) {
                            Text(u.name).font(.system(size: 12, weight: .medium)).lineLimit(1).truncationMode(.middle)
                            Bar(fraction: u.bytes > 0 ? u.sent / u.bytes : 0, active: u.state == "uploading")
                            Text(line(for: u)).font(.system(size: 10.5)).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                    if m.uploads.count > 4 {
                        Text("+ \(m.uploads.count - 4) more").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                    if !m.recent.isEmpty {
                        Divider()
                        ForEach(m.recent) { d in
                            HStack(spacing: 6) {
                                Image(systemName: d.failed ? "exclamationmark.triangle.fill" : d.verified == true ? "checkmark.seal.fill" : "checkmark.circle")
                                    .foregroundStyle(d.failed ? Color.orange : Color.green).font(.system(size: 11))
                                Text(d.name).font(.system(size: 11.5)).lineLimit(1).truncationMode(.middle)
                                Spacer()
                                Text(ago(d.finishedAt)).font(.system(size: 10.5)).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }

            HStack(spacing: 8) {
                PillButton(title: "Open Drive", symbol: "folder", prominent: !m.needsSignIn && m.mounted) { m.openDrive() }
                    .disabled(!m.mounted)
                PillButton(title: "Dashboard", symbol: "gauge.with.dots.needle.33percent") { m.openDashboard() }
                    .disabled(!m.reachable)
            }

            HStack {
                Text(m.cache).font(.system(size: 10.5)).foregroundStyle(.secondary)
                Spacer()
                Button(m.openAtLogin ? "✓ Open at Login" : "Open at Login") { m.toggleOpenAtLogin() }
                    .buttonStyle(.plain).font(.system(size: 10.5)).foregroundStyle(.secondary)
                Text("·").foregroundStyle(.secondary).font(.system(size: 10.5))
                Button("Quit") { NSApp.terminate(nil) }
                    .buttonStyle(.plain).font(.system(size: 10.5)).foregroundStyle(.secondary)
            }
        }
        .padding(14)
        .frame(width: 320)
    }

    func line(for u: Upload) -> String {
        if u.state == "uploading" {
            let v = m.speed(of: u)
            var s = "\(fmtBytes(u.sent)) of \(fmtBytes(u.bytes))"
            if v > 1e4 { s += String(format: " · %.1f MB/s · %@ left", v / 1e6, fmtDuration((u.bytes - u.sent) / v)) }
            else if !u.detail.isEmpty { s += " · " + u.detail }
            return s
        }
        return "\(fmtBytes(u.bytes)) · \(u.detail)"
    }

    func ago(_ t: Double) -> String {
        let s = Date().timeIntervalSince1970 - t
        return s < 45 ? "just now" : fmtDuration(s) + " ago"
    }
}

@main
struct DriveStreamApp: App {
    @StateObject private var model = Model()

    init() {
        // `DriveStream --snapshot out.png`: render the panel with live data and exit (for testing).
        let args = CommandLine.arguments
        if let i = args.firstIndex(of: "--snapshot"), i + 1 < args.count {
            let out = URL(fileURLWithPath: args[i + 1])
            Task { @MainActor in
                let m = Model()
                await m.poll()
                try? await Task.sleep(nanoseconds: 2_100_000_000)
                await m.poll()  // a second sample so speeds and ETAs appear
                let r = ImageRenderer(content: Panel().environmentObject(m).background(Color(nsColor: .windowBackgroundColor)))
                r.scale = 2
                if let img = r.nsImage, let tiff = img.tiffRepresentation,
                   let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) {
                    try? png.write(to: out)
                }
                exit(0)
            }
        }
    }

    var body: some Scene {
        MenuBarExtra {
            Panel().environmentObject(model)
                .onAppear { Task { await model.poll() } }
        } label: {
            Image(systemName: model.icon)
                .task { model.start() }
        }
        .menuBarExtraStyle(.window)
    }
}
