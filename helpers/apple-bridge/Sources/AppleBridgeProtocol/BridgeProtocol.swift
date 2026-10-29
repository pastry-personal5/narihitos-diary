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

public enum AssistantModelAvailability: String, Sendable {
    case available
    case deviceNotEligible = "device-not-eligible"
    case appleIntelligenceNotEnabled = "apple-intelligence-not-enabled"
    case modelNotReady = "model-not-ready"
}

public struct AssistantSource: Sendable {
    public let ordinal: Int
    public let body: String

    public init(ordinal: Int, body: String) {
        self.ordinal = ordinal
        self.body = body
    }
}

public enum AssistantRequest: Sendable {
    case extract(locale: String, sources: [AssistantSource])
    case draft(locale: String, reviewedMemory: String, priorDraft: String?, style: String)
    case reviewParagraph(locale: String, paragraph: String, style: String)
    case refineSource(locale: String, body: String)
}

public struct AssistantReflectionRange: Sendable {
    public let start: Int
    public let end: Int

    public init(start: Int, end: Int) {
        self.start = start
        self.end = end
    }
}

public struct AssistantMemoryCandidate: Sendable {
    public let text: String
    public let sourceOrdinals: [Int]

    public init(text: String, sourceOrdinals: [Int]) {
        self.text = text
        self.sourceOrdinals = sourceOrdinals
    }
}

public enum AssistantResult: Sendable {
    case memories([AssistantMemoryCandidate])
    case draft(text: String, reflectionRanges: [AssistantReflectionRange])
    case paragraph(
        replacement: String,
        reflectionRanges: [AssistantReflectionRange],
        explanation: String?
    )
    case sourceRefinement(body: String)
}

public protocol AssistantService {
    func availability(locale: String) -> AssistantModelAvailability
    func perform(_ request: AssistantRequest) throws -> AssistantResult
}

public enum AssistantServiceError: String, Error, Sendable {
    case unsupportedLocale = "unsupported_locale"
    case refusal
    case contextOverflow = "context_overflow"
    case invalidOutput = "invalid_output"
    case timedOut = "timed_out"
    case cancelled
    case unavailable
}

public enum BridgeProtocol {
    public static let version = 2
    public static let maximumFrameBytes = 20 * 1024 * 1024
    public static let maximumRequestIDBytes = 128

    public static func handle(
        _ input: Data,
        sourceService: SourceService? = nil,
        assistantService: AssistantService? = nil
    ) -> BridgeOutcome {
        guard input.count >= 4 else {
            return failure(code: "malformed_input", requestID: nil)
        }
        let declaredLength = input.prefix(4).reduce(0) { ($0 << 8) | Int($1) }
        guard declaredLength <= maximumFrameBytes else {
            return failure(code: "frame_too_large", requestID: nil)
        }
        guard input.count == declaredLength + 4 else {
            return failure(code: "malformed_input", requestID: nil)
        }

        let frame = Data(input.dropFirst(4))
        guard !frame.isEmpty,
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
        if command.hasPrefix("assistant-") {
            return handleAssistant(
                command: command,
                payload: payload,
                requestID: requestID,
                service: assistantService
            )
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

    private static func handleAssistant(
        command: String,
        payload: [String: Any],
        requestID: String,
        service: AssistantService?
    ) -> BridgeOutcome {
        guard let service else {
            return failure(code: "unknown_command", requestID: requestID)
        }
        guard let locale = payload["locale"] as? String,
              locale == "en-US" || locale == "ko-KR"
        else {
            return failure(code: "unsupported_locale", requestID: requestID)
        }
        if command == "assistant-availability" {
            guard payload.keys.sorted() == ["locale"] else {
                return failure(code: "malformed_input", requestID: requestID)
            }
            return success(
                requestID: requestID,
                value: ["status": service.availability(locale: locale).rawValue]
            )
        }
        let request: AssistantRequest
        switch command {
        case "assistant-extract-memories":
            guard payload.keys.sorted() == ["locale", "sources"],
                  let rawSources = payload["sources"] as? [[String: Any]],
                  !rawSources.isEmpty,
                  rawSources.count <= 64
            else {
                return failure(code: "malformed_input", requestID: requestID)
            }
            var sources: [AssistantSource] = []
            var totalBytes = 0
            for raw in rawSources {
                guard raw.keys.sorted() == ["body", "ordinal"],
                      let ordinal = integer(raw["ordinal"]),
                      ordinal == sources.count + 1,
                      let body = raw["body"] as? String,
                      !body.isEmpty,
                      body.utf8.count <= 8 * 1024 * 1024
                else {
                    return failure(code: "malformed_input", requestID: requestID)
                }
                totalBytes += body.utf8.count
                guard totalBytes <= 16 * 1024 * 1024 else {
                    return failure(code: "frame_too_large", requestID: requestID)
                }
                sources.append(AssistantSource(ordinal: ordinal, body: body))
            }
            request = .extract(locale: locale, sources: sources)
        case "assistant-draft-entry":
            guard payload.keys.sorted() == ["locale", "prior_draft", "reviewed_memory", "style"],
                  let memory = payload["reviewed_memory"] as? String,
                  memory.utf8.count <= 512 * 1024,
                  let style = validStyle(payload["style"]),
                  payload["prior_draft"] is NSNull || payload["prior_draft"] is String
            else {
                return failure(code: "malformed_input", requestID: requestID)
            }
            let priorDraft = payload["prior_draft"] as? String
            guard priorDraft?.utf8.count ?? 0 <= 512 * 1024 else {
                return failure(code: "frame_too_large", requestID: requestID)
            }
            request = .draft(
                locale: locale,
                reviewedMemory: memory,
                priorDraft: priorDraft,
                style: style
            )
        case "assistant-review-paragraph":
            guard payload.keys.sorted() == ["locale", "paragraph", "style"],
                  let paragraph = payload["paragraph"] as? String,
                  !paragraph.isEmpty,
                  paragraph.utf8.count <= 32 * 1024,
                  let style = validStyle(payload["style"])
            else {
                return failure(code: "malformed_input", requestID: requestID)
            }
            request = .reviewParagraph(locale: locale, paragraph: paragraph, style: style)
        case "assistant-refine-source":
            guard payload.keys.sorted() == ["body", "locale"],
                  let body = payload["body"] as? String,
                  !body.isEmpty,
                  body.utf8.count <= 8 * 1024 * 1024
            else {
                return failure(code: "malformed_input", requestID: requestID)
            }
            request = .refineSource(locale: locale, body: body)
        default:
            return failure(code: "unknown_command", requestID: requestID)
        }
        do {
            let result = try service.perform(request)
            guard let value = validatedAssistantValue(result, request: request) else {
                return failure(code: "invalid_output", requestID: requestID)
            }
            return success(requestID: requestID, value: value)
        } catch let error as AssistantServiceError {
            return failure(code: error.rawValue, requestID: requestID)
        } catch {
            return failure(code: "unavailable", requestID: requestID)
        }
    }

    private static func validStyle(_ value: Any?) -> String? {
        guard let value = value as? String,
              ["concise", "standard", "reflective"].contains(value)
        else { return nil }
        return value
    }

    private static func validatedAssistantValue(
        _ result: AssistantResult,
        request: AssistantRequest
    ) -> [String: Any]? {
        switch (request, result) {
        case let (.extract(_, sources), .memories(candidates)):
            guard candidates.count <= 128 else { return nil }
            var values: [[String: Any]] = []
            for candidate in candidates {
                let ordinals = candidate.sourceOrdinals
                guard !candidate.text.isEmpty,
                      candidate.text.utf8.count <= 4 * 1024,
                      !ordinals.isEmpty,
                      ordinals.count <= 64,
                      Set(ordinals).count == ordinals.count,
                      ordinals.allSatisfy({ $0 > 0 && $0 <= sources.count })
                else { return nil }
                values.append(["text": candidate.text, "source_ordinals": ordinals])
            }
            return ["candidates": values]
        case let (.draft(_, _, _, _), .draft(text, ranges)):
            guard validStructuredText(text, ranges: ranges, maximumBytes: 512 * 1024) else {
                return nil
            }
            return ["text": text, "reflection_ranges": rangeValues(ranges)]
        case let (.reviewParagraph(_, _, _), .paragraph(replacement, ranges, explanation)):
            guard validStructuredText(replacement, ranges: ranges, maximumBytes: 32 * 1024),
                  explanation?.utf8.count ?? 0 <= 4 * 1024
            else { return nil }
            return [
                "replacement": replacement,
                "reflection_ranges": rangeValues(ranges),
                "explanation": (explanation as Any?) ?? NSNull(),
            ]
        case let (.refineSource(_, _), .sourceRefinement(body)):
            guard !body.isEmpty, body.utf8.count <= 8 * 1024 * 1024 else { return nil }
            return ["body": body]
        default:
            return nil
        }
    }

    private static func validStructuredText(
        _ text: String,
        ranges: [AssistantReflectionRange],
        maximumBytes: Int
    ) -> Bool {
        guard !text.isEmpty,
              text.utf8.count <= maximumBytes,
              ranges.count <= 32,
              !text.contains("⟪"),
              !text.contains("⟫")
        else { return false }
        var previousEnd = 0
        for range in ranges {
            guard range.start >= previousEnd,
                  range.start < range.end,
                  range.end <= text.utf8.count,
                  utf8Boundary(range.start, in: text),
                  utf8Boundary(range.end, in: text)
            else { return false }
            previousEnd = range.end
        }
        return text.utf8.count + ranges.count * 6 <= maximumBytes
    }

    private static func utf8Boundary(_ offset: Int, in text: String) -> Bool {
        let index = text.utf8.index(text.utf8.startIndex, offsetBy: offset)
        return index.samePosition(in: text) != nil
    }

    private static func rangeValues(_ ranges: [AssistantReflectionRange]) -> [[String: Int]] {
        ranges.map { ["start": $0.start, "end": $0.end] }
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
            let body = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
            let response = framed(body)
            return BridgeOutcome(
                response: response,
                diagnosticCode: diagnosticCode,
                succeeded: succeeded
            )
        } catch {
            let fallback = framed(Data(
                #"{"error":{"code":"internal_error","message":"request rejected"},"ok":false,"protocol_version":2,"request_id":null}"#.utf8
            ))
            return BridgeOutcome(
                response: fallback,
                diagnosticCode: "internal_error",
                succeeded: false
            )
        }
    }

    private static func framed(_ body: Data) -> Data {
        let length = UInt32(body.count)
        var response = Data([
            UInt8((length >> 24) & 0xff),
            UInt8((length >> 16) & 0xff),
            UInt8((length >> 8) & 0xff),
            UInt8(length & 0xff),
        ])
        response.append(body)
        return response
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
