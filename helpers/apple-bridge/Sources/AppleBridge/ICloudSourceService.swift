import AppleBridgeProtocol
import Darwin
import Foundation

struct LiveICloudSourceService: SourceService {
    private let fileManager = FileManager.default

    func availability(directoryName: String, sourceID: String) throws -> SourceAvailability {
        let (_, source) = try sourceLocation(directoryName: directoryName, sourceID: sourceID)
        var sourceStat = stat()
        if lstat(source.path, &sourceStat) != 0 {
            switch errno {
            case ENOENT: return .missing
            case EACCES, EPERM: return .permissionDenied
            default: throw SourceServiceError.unavailable
            }
        }
        guard sourceStat.st_mode & S_IFMT == S_IFREG else {
            throw SourceServiceError.invalidSource
        }
        do {
            let values = try source.resourceValues(forKeys: [
                .isUbiquitousItemKey,
                .ubiquitousItemDownloadingStatusKey,
            ])
            if values.isUbiquitousItem == true,
               values.ubiquitousItemDownloadingStatus != .current
            {
                return .cloudOnly
            }
            return .resident
        } catch CocoaError.fileReadNoPermission {
            return .permissionDenied
        } catch {
            throw SourceServiceError.unavailable
        }
    }

    func requestDownload(
        directoryName: String,
        sourceID: String,
        timeoutMilliseconds: Int
    ) throws -> SourceAvailability {
        let (_, source) = try sourceLocation(directoryName: directoryName, sourceID: sourceID)
        switch try availability(directoryName: directoryName, sourceID: sourceID) {
        case .resident: return .resident
        case .missing: throw SourceServiceError.notFound
        case .permissionDenied: throw SourceServiceError.permissionDenied
        case .cloudOnly: break
        }
        do {
            try fileManager.startDownloadingUbiquitousItem(at: source)
        } catch CocoaError.fileReadNoPermission {
            throw SourceServiceError.permissionDenied
        } catch {
            throw SourceServiceError.unavailable
        }
        let deadline = Date().addingTimeInterval(Double(timeoutMilliseconds) / 1_000)
        while Date() < deadline {
            let state = try availability(directoryName: directoryName, sourceID: sourceID)
            if state != .cloudOnly {
                return state
            }
            Thread.sleep(forTimeInterval: 0.05)
        }
        return .cloudOnly
    }

    func coordinatedRead(
        directoryName: String,
        sourceID: String,
        handoffDirectory: String,
        maximumBytes: Int
    ) throws -> CoordinatedReadResult {
        let (base, source) = try sourceLocation(directoryName: directoryName, sourceID: sourceID)
        switch try availability(directoryName: directoryName, sourceID: sourceID) {
        case .resident: break
        case .cloudOnly: throw SourceServiceError.needsDownload
        case .missing: throw SourceServiceError.notFound
        case .permissionDenied: throw SourceServiceError.permissionDenied
        }
        let handoff = try validatedHandoffDirectory(handoffDirectory)
        var coordinationError: NSError?
        var result: Result<(Data, stat), Error>?
        NSFileCoordinator(filePresenter: nil).coordinate(
            readingItemAt: source,
            options: [],
            error: &coordinationError
        ) { coordinatedURL in
            do {
                let resolvedURL = coordinatedURL.resolvingSymlinksInPath()
                guard contained(resolvedURL, in: base) else {
                    throw SourceServiceError.invalidSource
                }
                var before = stat()
                guard lstat(resolvedURL.path, &before) == 0,
                      before.st_mode & S_IFMT == S_IFREG
                else {
                    throw SourceServiceError.unavailable
                }
                guard before.st_size >= 0, Int(before.st_size) <= maximumBytes else {
                    throw SourceServiceError.oversize
                }
                let data = try Data(contentsOf: resolvedURL, options: [.mappedIfSafe])
                var after = stat()
                guard lstat(resolvedURL.path, &after) == 0,
                      sameFile(before, after),
                      data.count == Int(after.st_size)
                else {
                    throw SourceServiceError.unavailable
                }
                result = .success((data, after))
            } catch {
                result = .failure(error)
            }
        }
        if let coordinationError {
            if coordinationError.code == NSFileReadNoPermissionError {
                throw SourceServiceError.permissionDenied
            }
            throw SourceServiceError.unavailable
        }
        guard let result else {
            throw SourceServiceError.unavailable
        }
        let (data, sourceStat) = try result.get()
        let handoffName = "nd-source-\(UUID().uuidString.lowercased()).bin"
        let handoffURL = handoff.appendingPathComponent(handoffName, isDirectory: false)
        guard fileManager.createFile(atPath: handoffURL.path, contents: data),
              chmod(handoffURL.path, S_IRUSR | S_IWUSR) == 0
        else {
            try? fileManager.removeItem(at: handoffURL)
            throw SourceServiceError.unavailable
        }
        return CoordinatedReadResult(
            handoffName: handoffName,
            fileIdentity: "\(sourceStat.st_dev):\(sourceStat.st_ino)",
            size: data.count,
            modifiedAt: Int(sourceStat.st_mtimespec.tv_sec)
        )
    }

    private func sourceLocation(directoryName: String, sourceID: String) throws -> (URL, URL) {
        guard validDirectoryName(directoryName), validSourceID(sourceID) else {
            throw SourceServiceError.invalidSource
        }
        let cloudRoot = fileManager.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Mobile Documents/com~apple~CloudDocs", isDirectory: true)
            .resolvingSymlinksInPath()
        let rawBase = cloudRoot.appendingPathComponent(directoryName, isDirectory: true)
        var baseStat = stat()
        if lstat(rawBase.path, &baseStat) == 0, baseStat.st_mode & S_IFMT == S_IFLNK {
            throw SourceServiceError.invalidSource
        }
        let base = rawBase.resolvingSymlinksInPath()
        guard contained(base, in: cloudRoot) else {
            throw SourceServiceError.invalidSource
        }
        let source = base.appendingPathComponent(sourceID, isDirectory: false)
        let resolvedSource = source.resolvingSymlinksInPath()
        guard contained(resolvedSource, in: base) else {
            throw SourceServiceError.invalidSource
        }
        return (base, source)
    }

    private func validatedHandoffDirectory(_ path: String) throws -> URL {
        let url = URL(fileURLWithPath: path, isDirectory: true)
        var directoryStat = stat()
        guard lstat(url.path, &directoryStat) == 0,
              directoryStat.st_mode & S_IFMT == S_IFDIR,
              directoryStat.st_mode & 0o777 == 0o700,
              directoryStat.st_uid == getuid()
        else {
            throw SourceServiceError.invalidSource
        }
        return url
    }
}

private func sameFile(_ left: stat, _ right: stat) -> Bool {
    left.st_dev == right.st_dev
        && left.st_ino == right.st_ino
        && left.st_size == right.st_size
        && left.st_mtimespec.tv_sec == right.st_mtimespec.tv_sec
        && left.st_mtimespec.tv_nsec == right.st_mtimespec.tv_nsec
}

private func contained(_ child: URL, in parent: URL) -> Bool {
    let childParts = child.standardizedFileURL.pathComponents
    let parentParts = parent.standardizedFileURL.pathComponents
    return childParts.count > parentParts.count
        && Array(childParts.prefix(parentParts.count)) == parentParts
}

private func validDirectoryName(_ value: String) -> Bool {
    !value.isEmpty
        && value != "."
        && value != ".."
        && !value.contains("/")
        && !value.contains("\\")
        && !value.contains("\0")
}

private func validSourceID(_ value: String) -> Bool {
    let parts = value.split(separator: "/", omittingEmptySubsequences: false)
    guard parts.count == 4,
          parts[0].count == 4,
          parts[1].count == 2,
          parts[2].count == 2,
          parts[0...2].allSatisfy({ part in
              part.utf8.allSatisfy { $0 >= 0x30 && $0 <= 0x39 }
          }),
          let year = Int(parts[0]),
          let month = Int(parts[1]),
          let day = Int(parts[2]),
          validDate(year: year, month: month, day: day),
          parts[3].hasSuffix(".md"),
          !parts[3].hasPrefix(".")
    else {
        return false
    }
    return true
}

private func validDate(year: Int, month: Int, day: Int) -> Bool {
    let maximum: Int
    switch month {
    case 1, 3, 5, 7, 8, 10, 12: maximum = 31
    case 4, 6, 9, 11: maximum = 30
    case 2 where year.isMultiple(of: 4)
        && (!year.isMultiple(of: 100) || year.isMultiple(of: 400)):
        maximum = 29
    case 2: maximum = 28
    default: return false
    }
    return day >= 1 && day <= maximum
}
