import CoreFoundation
import Foundation

public struct BridgeOutcome: Sendable {
    public let response: Data
    public let diagnosticCode: String?
    public let succeeded: Bool
}

public enum BridgeProtocol {
    public static let version = 1
    public static let maximumFrameBytes = 64 * 1024
    public static let maximumRequestIDBytes = 128

    private static let newline: UInt8 = 0x0A

    public static func handle(_ input: Data) -> BridgeOutcome {
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
        guard command == "ping" else {
            return failure(code: "unknown_command", requestID: requestID)
        }

        return success(requestID: requestID, value: ["status": "ready"])
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
}
