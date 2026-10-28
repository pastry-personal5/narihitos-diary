import CoreFoundation
import Foundation

public struct BridgeOutcome: Sendable {
    public let response: Data
    public let diagnosticCode: String?
    public let succeeded: Bool
}

public enum SourceAvailability: String, Sendable {
    case resident
    case cloudOnly = "cloud-only"
    case missing
    case permissionDenied = "permission-denied"
}

public struct CoordinatedReadResult: Sendable {
    public let handoffName: String
    public let fileIdentity: String
    public let size: Int
    public let modifiedAt: Int

    public init(handoffName: String, fileIdentity: String, size: Int, modifiedAt: Int) {
        self.handoffName = handoffName
        self.fileIdentity = fileIdentity
        self.size = size
        self.modifiedAt = modifiedAt
    }
}

public enum SourceServiceError: String, Error, Sendable {
    case invalidSource = "invalid_source"
    case permissionDenied = "permission_denied"
    case needsDownload = "needs_download"
    case notFound = "not_found"
    case oversize
    case timedOut = "timed_out"
    case cancelled
    case unavailable
}

public protocol SourceService {
    func availability(directoryName: String, sourceID: String) throws -> SourceAvailability
    func requestDownload(
        directoryName: String,
        sourceID: String,
        timeoutMilliseconds: Int
    ) throws -> SourceAvailability
    func coordinatedRead(
        directoryName: String,
        sourceID: String,
        handoffDirectory: String,
        maximumBytes: Int
    ) throws -> CoordinatedReadResult
}

public enum BridgeProtocol {
    public static let version = 1
    public static let maximumFrameBytes = 64 * 1024
    public static let maximumRequestIDBytes = 128

    private static let newline: UInt8 = 0x0A

    public static func handle(_ input: Data, sourceService: SourceService? = nil) -> BridgeOutcome {
        guard input.count <= maximumFrameBytes else {
            return failure(code: "frame_too_large", requestID: nil)
        }
        guard input.last == newline else {
            return failure(code: "malformed_input", requestID: nil)
        }

        let frame = Data(input.dropLast())
        guard !frame.isEmpty,
              !frame.contains(newline),
              String(data: frame, encoding: .utf8) != nil
        else {
            return failure(code: "malformed_input", requestID: nil)
        }

        let decoded: Any
        do {
            decoded = try JSONSerialization.jsonObject(with: frame)
        } catch {
            return failure(code: "malformed_input", requestID: nil)
        }

        guard let request = decoded as? [String: Any] else {
            return failure(code: "malformed_input", requestID: nil)
        }

        let requestID: String?
        if let candidate = request["request_id"] as? String,
           !candidate.isEmpty,
           candidate.utf8.count <= maximumRequestIDBytes
        {
            requestID = candidate
        } else {
            requestID = nil
        }
        guard let requestID,
              let rawVersion = request["protocol_version"] as? NSNumber,
              CFGetTypeID(rawVersion) != CFBooleanGetTypeID(),
              !CFNumberIsFloatType(rawVersion),
              let command = request["command"] as? String,
              request["payload"] is [String: Any]
        else {
            return failure(code: "malformed_input", requestID: requestID)
        }

        let requestVersion = rawVersion.intValue
        guard requestVersion == version else {
            return failure(code: "unsupported_version", requestID: requestID)
        }
        let payload = request["payload"] as! [String: Any]
        if command == "ping" {
            return success(requestID: requestID, value: ["status": "ready"])
        }
        guard let sourceService else {
            return failure(code: "unknown_command", requestID: requestID)
        }
        guard let directoryName = payload["directory_name"] as? String,
              let sourceID = payload["source_id"] as? String,
              validDirectoryName(directoryName),
              validSourceID(sourceID)
        else {
            return failure(code: "malformed_input", requestID: requestID)
        }
        do {
            switch command {
            case "icloud-availability":
                let availability = try sourceService.availability(
                    directoryName: directoryName,
                    sourceID: sourceID
                )
                return success(requestID: requestID, value: ["status": availability.rawValue])
            case "icloud-request-download":
                guard let timeout = integer(payload["timeout_milliseconds"]),
                      timeout > 0,
                      timeout <= 60_000
                else {
                    return failure(code: "malformed_input", requestID: requestID)
                }
                let availability = try sourceService.requestDownload(
                    directoryName: directoryName,
                    sourceID: sourceID,
                    timeoutMilliseconds: timeout
                )
                return success(requestID: requestID, value: ["status": availability.rawValue])
            case "icloud-coordinated-read":
                guard let handoffDirectory = payload["handoff_directory"] as? String,
                      !handoffDirectory.isEmpty,
                      let maximumBytes = integer(payload["maximum_bytes"]),
                      maximumBytes > 0,
                      maximumBytes <= 8 * 1024 * 1024
                else {
                    return failure(code: "malformed_input", requestID: requestID)
                }
                let result = try sourceService.coordinatedRead(
                    directoryName: directoryName,
                    sourceID: sourceID,
                    handoffDirectory: handoffDirectory,
                    maximumBytes: maximumBytes
                )
                return success(
                    requestID: requestID,
                    value: [
                        "handoff_name": result.handoffName,
                        "file_identity": result.fileIdentity,
                        "size": result.size,
                        "modified_at": result.modifiedAt,
                    ]
                )
            default:
                return failure(code: "unknown_command", requestID: requestID)
            }
        } catch let error as SourceServiceError {
            return failure(code: error.rawValue, requestID: requestID)
        } catch {
            return failure(code: "unavailable", requestID: requestID)
        }
    }

    private static func success(requestID: String, value: [String: Any]) -> BridgeOutcome {
        encode(
            object: [
                "protocol_version": version,
                "request_id": requestID,
                "ok": true,
                "value": value,
            ],
            diagnosticCode: nil,
            succeeded: true
        )
    }

    private static func failure(code: String, requestID: String?) -> BridgeOutcome {
        encode(
            object: [
                "protocol_version": version,
                "request_id": (requestID as Any?) ?? NSNull(),
                "ok": false,
                "error": [
                    "code": code,
                    "message": "request rejected",
                ],
            ],
            diagnosticCode: code,
            succeeded: false
        )
    }

    private static func encode(
        object: [String: Any],
        diagnosticCode: String?,
        succeeded: Bool
    ) -> BridgeOutcome {
        do {
            var response = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
            response.append(newline)
            return BridgeOutcome(
                response: response,
                diagnosticCode: diagnosticCode,
                succeeded: succeeded
            )
        } catch {
            let fallback = Data(
                #"{"error":{"code":"internal_error","message":"request rejected"},"ok":false,"protocol_version":1,"request_id":null}"#.utf8
            ) + Data([newline])
            return BridgeOutcome(
                response: fallback,
                diagnosticCode: "internal_error",
                succeeded: false
            )
        }
    }

    private static func integer(_ value: Any?) -> Int? {
        guard let number = value as? NSNumber,
              CFGetTypeID(number) != CFBooleanGetTypeID(),
              !CFNumberIsFloatType(number)
        else {
            return nil
        }
        return number.intValue
    }

    private static func validDirectoryName(_ value: String) -> Bool {
        !value.isEmpty
            && value != "."
            && value != ".."
            && !value.contains("/")
            && !value.contains("\\")
            && !value.contains("\0")
    }

    private static func validSourceID(_ value: String) -> Bool {
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

    private static func validDate(year: Int, month: Int, day: Int) -> Bool {
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
}
