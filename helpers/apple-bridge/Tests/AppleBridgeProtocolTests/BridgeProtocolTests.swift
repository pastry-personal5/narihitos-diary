import Foundation
import XCTest
@testable import AppleBridgeProtocol

final class BridgeProtocolTests: XCTestCase {
    func testValidExchangeIsCorrelatedAndEmitsOneProtocolLine() throws {
        let outcome = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"request-7","command":"ping","payload":{}}"#
            )
        )
        let response = try decodedResponse(outcome)

        XCTAssertTrue(outcome.succeeded)
        XCTAssertNil(outcome.diagnosticCode)
        XCTAssertEqual(response["protocol_version"] as? Int, 2)
        XCTAssertEqual(response["request_id"] as? String, "request-7")
        XCTAssertEqual(response["ok"] as? Bool, true)
        let value = response["value"] as? [String: String]
        XCTAssertEqual(value?["status"], "ready")
        expectExactlyOneProtocolLine(outcome.response)
    }

    func testUnsupportedVersionFailsWithoutEchoingPayload() throws {
        let outcome = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":1,"request_id":"request-8","command":"ping","payload":{"private":"do-not-echo"}}"#
            )
        )

        try expectFailure(
            outcome,
            code: "unsupported_version",
            requestID: "request-8"
        )
        XCTAssertFalse(String(decoding: outcome.response, as: UTF8.self).contains("do-not-echo"))
    }

    func testUnknownCommandFailsWithStableCode() throws {
        let outcome = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"request-9","command":"future-command","payload":{}}"#
            )
        )

        try expectFailure(outcome, code: "unknown_command", requestID: "request-9")
    }

    func testMalformedInputFailsWithoutRequestCorrelation() throws {
        let outcome = BridgeProtocol.handle(frame(#"{"protocol_version":2"#))

        try expectFailure(outcome, code: "malformed_input", requestID: nil)
    }

    func testInvalidVersionTypeAndOversizedRequestIDFailAsMalformed() throws {
        let booleanVersion = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":true,"request_id":"request-10","command":"ping","payload":{}}"#
            )
        )
        try expectFailure(booleanVersion, code: "malformed_input", requestID: "request-10")

        let oversizedRequestID = String(
            repeating: "x",
            count: BridgeProtocol.maximumRequestIDBytes + 1
        )
        let oversizedID = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"\#(oversizedRequestID)","command":"ping","payload":{}}"#
            )
        )
        try expectFailure(oversizedID, code: "malformed_input", requestID: nil)
    }

    func testUnterminatedAndMultipleFramesFailClosed() throws {
        let unterminated = BridgeProtocol.handle(Data([0, 0, 0, 3]) + Data(#"{}"#.utf8))
        try expectFailure(unterminated, code: "malformed_input", requestID: nil)

        let multiple = BridgeProtocol.handle(frame(#"{}"#) + frame(#"{}"#))
        try expectFailure(multiple, code: "malformed_input", requestID: nil)
    }

    func testOversizedFrameFailsBeforeDecoding() throws {
        let body = Data(repeating: 0x20, count: BridgeProtocol.maximumFrameBytes + 1)
        let input = frame(body)
        let outcome = BridgeProtocol.handle(input)

        try expectFailure(outcome, code: "frame_too_large", requestID: nil)
    }

    func testFrameAtExactLimitIsAccepted() throws {
        let prefix = #"{"protocol_version":2,"request_id":"boundary","command":"ping","payload":{"padding":""#
        let suffix = #""}}"#
        let paddingCount = BridgeProtocol.maximumFrameBytes
            - Data(prefix.utf8).count
            - Data(suffix.utf8).count
        let json = prefix + String(repeating: "x", count: paddingCount) + suffix
        let input = frame(json)

        XCTAssertEqual(input.count, BridgeProtocol.maximumFrameBytes + 4)
        let outcome = BridgeProtocol.handle(input)
        let response = try decodedResponse(outcome)
        XCTAssertTrue(outcome.succeeded)
        XCTAssertEqual(response["request_id"] as? String, "boundary")
        expectExactlyOneProtocolLine(outcome.response)
    }

    func testICloudCommandsUseSelectedSourceServiceAndReturnBoundedMetadata() throws {
        let service = FakeSourceService()
        let availability = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"a","command":"icloud-availability","payload":{"directory_name":"Ayumi","source_id":"2026/10/29/n.md"}}"#
            ),
            sourceService: service
        )
        let availabilityValue = try XCTUnwrap(
            decodedResponse(availability)["value"] as? [String: Any]
        )
        XCTAssertEqual(availabilityValue["status"] as? String, "cloud-only")

        let download = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"d","command":"icloud-request-download","payload":{"directory_name":"Ayumi","source_id":"2026/10/29/n.md","timeout_milliseconds":5000}}"#
            ),
            sourceService: service
        )
        let downloadValue = try XCTUnwrap(decodedResponse(download)["value"] as? [String: Any])
        XCTAssertEqual(downloadValue["status"] as? String, "resident")

        let read = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"r","command":"icloud-coordinated-read","payload":{"directory_name":"Ayumi","source_id":"2026/10/29/n.md","handoff_directory":"/synthetic/private","maximum_bytes":8388608}}"#
            ),
            sourceService: service
        )
        let readValue = try XCTUnwrap(decodedResponse(read)["value"] as? [String: Any])
        XCTAssertEqual(readValue["handoff_name"] as? String, "nd-source-fake.bin")
        XCTAssertEqual(readValue["file_identity"] as? String, "1:2")
        XCTAssertEqual(readValue["size"] as? Int, 70_000)
        XCTAssertEqual(service.calls, ["availability", "download", "read"])
    }

    func testICloudCommandsRejectUnsafeInputsAndReturnStableServiceErrors() throws {
        let service = FakeSourceService()
        let unsafe = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"u","command":"icloud-availability","payload":{"directory_name":"../Ayumi","source_id":"2026/10/29/n.md"}}"#
            ),
            sourceService: service
        )
        try expectFailure(unsafe, code: "malformed_input", requestID: "u")

        for sourceID in ["2026/13/29/n.md", "2026/02/30/n.md", "２０２６/10/29/n.md"] {
            let invalidDate = BridgeProtocol.handle(
                frame(
                    #"{"protocol_version":2,"request_id":"u","command":"icloud-availability","payload":{"directory_name":"Ayumi","source_id":"\#(sourceID)"}}"#
                ),
                sourceService: service
            )
            try expectFailure(invalidDate, code: "malformed_input", requestID: "u")
        }

        service.error = .permissionDenied
        let denied = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"p","command":"icloud-availability","payload":{"directory_name":"Ayumi","source_id":"2026/10/29/n.md"}}"#
            ),
            sourceService: service
        )
        try expectFailure(denied, code: "permission_denied", requestID: "p")
        XCTAssertFalse(String(decoding: denied.response, as: UTF8.self).contains("Ayumi"))
    }

    func testAssistantCommandsCoverBothLocalesAndAllActions() throws {
        let service = FakeAssistantService()
        let availability = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"a","command":"assistant-availability","payload":{"locale":"ko-KR"}}"#
            ),
            assistantService: service
        )
        let availabilityValue = try XCTUnwrap(
            decodedResponse(availability)["value"] as? [String: Any]
        )
        XCTAssertEqual(availabilityValue["status"] as? String, "available")

        let extraction = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"e","command":"assistant-extract-memories","payload":{"locale":"en-US","sources":[{"ordinal":1,"body":"untrusted source"}]}}"#
            ),
            assistantService: service
        )
        let extractionValue = try XCTUnwrap(
            decodedResponse(extraction)["value"] as? [String: Any]
        )
        XCTAssertEqual((extractionValue["candidates"] as? [[String: Any]])?.count, 1)

        let draft = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"d","command":"assistant-draft-entry","payload":{"locale":"ko-KR","prior_draft":null,"reviewed_memory":"memory","style":"reflective"}}"#
            ),
            assistantService: service
        )
        let draftValue = try XCTUnwrap(decodedResponse(draft)["value"] as? [String: Any])
        XCTAssertEqual(draftValue["text"] as? String, "draft")

        let paragraph = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"p","command":"assistant-review-paragraph","payload":{"locale":"en-US","paragraph":"paragraph","style":"standard"}}"#
            ),
            assistantService: service
        )
        let paragraphValue = try XCTUnwrap(
            decodedResponse(paragraph)["value"] as? [String: Any]
        )
        XCTAssertEqual(paragraphValue["replacement"] as? String, "replacement")
        let refinement = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"r","command":"assistant-refine-source","payload":{"locale":"en-US","body":"source body"}}"#
            ),
            assistantService: service
        )
        let refinementValue = try XCTUnwrap(
            decodedResponse(refinement)["value"] as? [String: Any]
        )
        XCTAssertEqual(refinementValue["body"] as? String, "refined source")
        XCTAssertEqual(service.actions, ["extract", "draft", "paragraph", "refine"])
    }

    func testAssistantCommandsRejectLocaleBoundsAndInvalidStructuredOutput() throws {
        let service = FakeAssistantService()
        let unsupported = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"u","command":"assistant-availability","payload":{"locale":"ja-JP"}}"#
            ),
            assistantService: service
        )
        try expectFailure(unsupported, code: "unsupported_locale", requestID: "u")

        let oversized = String(repeating: "x", count: 32 * 1024 + 1)
        let oversizedInput = try JSONSerialization.data(withJSONObject: [
            "protocol_version": 2,
            "request_id": "o",
            "command": "assistant-review-paragraph",
            "payload": ["locale": "en-US", "paragraph": oversized, "style": "standard"],
        ], options: [.sortedKeys])
        let rejected = BridgeProtocol.handle(frame(oversizedInput), assistantService: service)
        try expectFailure(rejected, code: "malformed_input", requestID: "o")

        service.invalidOutput = true
        let invalid = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"i","command":"assistant-extract-memories","payload":{"locale":"en-US","sources":[{"ordinal":1,"body":"source"}]}}"#
            ),
            assistantService: service
        )
        try expectFailure(invalid, code: "invalid_output", requestID: "i")
    }

    private func frame(_ json: String) -> Data {
        frame(Data(json.utf8))
    }

    private func frame(_ body: Data) -> Data {
        let length = UInt32(body.count)
        return Data([
            UInt8((length >> 24) & 0xff),
            UInt8((length >> 16) & 0xff),
            UInt8((length >> 8) & 0xff),
            UInt8(length & 0xff),
        ]) + body
    }

    private func decodedResponse(_ outcome: BridgeOutcome) throws -> [String: Any] {
        let length = outcome.response.prefix(4).reduce(0) { ($0 << 8) | Int($1) }
        XCTAssertEqual(outcome.response.count, length + 4)
        let object = try JSONSerialization.jsonObject(with: Data(outcome.response.dropFirst(4)))
        return try XCTUnwrap(object as? [String: Any])
    }

    private func expectFailure(
        _ outcome: BridgeOutcome,
        code: String,
        requestID: String?
    ) throws {
        let response = try decodedResponse(outcome)
        let error = try XCTUnwrap(response["error"] as? [String: String])

        XCTAssertFalse(outcome.succeeded)
        XCTAssertEqual(outcome.diagnosticCode, code)
        XCTAssertEqual(response["ok"] as? Bool, false)
        XCTAssertEqual(error["code"], code)
        XCTAssertEqual(error["message"], "request rejected")
        if let requestID {
            XCTAssertEqual(response["request_id"] as? String, requestID)
        } else {
            XCTAssertTrue(response["request_id"] is NSNull)
        }
        expectExactlyOneProtocolLine(outcome.response)
    }

    private func expectExactlyOneProtocolLine(_ response: Data) {
        XCTAssertLessThanOrEqual(response.count, BridgeProtocol.maximumFrameBytes + 4)
        let length = response.prefix(4).reduce(0) { ($0 << 8) | Int($1) }
        XCTAssertEqual(response.count, length + 4)
    }
}

private final class FakeSourceService: SourceService {
    var calls: [String] = []
    var error: SourceServiceError?

    func availability(directoryName: String, sourceID: String) throws -> SourceAvailability {
        calls.append("availability")
        if let error { throw error }
        return .cloudOnly
    }

    func requestDownload(
        directoryName: String,
        sourceID: String,
        timeoutMilliseconds: Int
    ) throws -> SourceAvailability {
        calls.append("download")
        if let error { throw error }
        return .resident
    }

    func coordinatedRead(
        directoryName: String,
        sourceID: String,
        handoffDirectory: String,
        maximumBytes: Int
    ) throws -> CoordinatedReadResult {
        calls.append("read")
        if let error { throw error }
        return CoordinatedReadResult(
            handoffName: "nd-source-fake.bin",
            fileIdentity: "1:2",
            size: 70_000,
            modifiedAt: 1_700_000_000
        )
    }
}

private final class FakeAssistantService: AssistantService {
    var actions: [String] = []
    var invalidOutput = false

    func availability(locale: String) -> AssistantModelAvailability {
        .available
    }

    func perform(_ request: AssistantRequest) throws -> AssistantResult {
        switch request {
        case .extract:
            actions.append("extract")
            return .memories([
                AssistantMemoryCandidate(
                    text: "memory",
                    sourceOrdinals: invalidOutput ? [2] : [1]
                ),
            ])
        case .draft:
            actions.append("draft")
            return .draft(text: "draft", reflectionRanges: [])
        case .reviewParagraph:
            actions.append("paragraph")
            return .paragraph(
                replacement: "replacement",
                reflectionRanges: [],
                explanation: "explanation"
            )
        case .refineSource:
            actions.append("refine")
            return .sourceRefinement(body: "refined source")
        }
    }
}
