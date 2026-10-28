import Foundation
import XCTest
@testable import AppleBridgeProtocol

final class BridgeProtocolTests: XCTestCase {
    func testValidExchangeIsCorrelatedAndEmitsOneProtocolLine() throws {
        let outcome = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":1,"request_id":"request-7","command":"ping","payload":{}}"#
            )
        )
        let response = try decodedResponse(outcome)

        XCTAssertTrue(outcome.succeeded)
        XCTAssertNil(outcome.diagnosticCode)
        XCTAssertEqual(response["protocol_version"] as? Int, 1)
        XCTAssertEqual(response["request_id"] as? String, "request-7")
        XCTAssertEqual(response["ok"] as? Bool, true)
        let value = response["value"] as? [String: String]
        XCTAssertEqual(value?["status"], "ready")
        expectExactlyOneProtocolLine(outcome.response)
    }

    func testUnsupportedVersionFailsWithoutEchoingPayload() throws {
        let outcome = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":2,"request_id":"request-8","command":"ping","payload":{"private":"do-not-echo"}}"#
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
                #"{"protocol_version":1,"request_id":"request-9","command":"future-command","payload":{}}"#
            )
        )

        try expectFailure(outcome, code: "unknown_command", requestID: "request-9")
    }

    func testMalformedInputFailsWithoutRequestCorrelation() throws {
        let outcome = BridgeProtocol.handle(frame(#"{"protocol_version":1"#))

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
                #"{"protocol_version":1,"request_id":"\#(oversizedRequestID)","command":"ping","payload":{}}"#
            )
        )
        try expectFailure(oversizedID, code: "malformed_input", requestID: nil)
    }

    func testUnterminatedAndMultipleFramesFailClosed() throws {
        let unterminated = BridgeProtocol.handle(Data(#"{}"#.utf8))
        try expectFailure(unterminated, code: "malformed_input", requestID: nil)

        let multiple = BridgeProtocol.handle(frame(#"{}"#) + frame(#"{}"#))
        try expectFailure(multiple, code: "malformed_input", requestID: nil)
    }

    func testOversizedFrameFailsBeforeDecoding() throws {
        let input = Data(repeating: 0x20, count: BridgeProtocol.maximumFrameBytes) + Data([0x0A])
        let outcome = BridgeProtocol.handle(input)

        try expectFailure(outcome, code: "frame_too_large", requestID: nil)
    }

    func testFrameAtExactLimitIsAccepted() throws {
        let prefix = #"{"protocol_version":1,"request_id":"boundary","command":"ping","payload":{"padding":""#
        let suffix = #""}}"#
        let paddingCount = BridgeProtocol.maximumFrameBytes
            - Data(prefix.utf8).count
            - Data(suffix.utf8).count
            - 1
        let json = prefix + String(repeating: "x", count: paddingCount) + suffix
        let input = frame(json)

        XCTAssertEqual(input.count, BridgeProtocol.maximumFrameBytes)
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
                #"{"protocol_version":1,"request_id":"a","command":"icloud-availability","payload":{"directory_name":"Ayumi","source_id":"2026/10/29/n.md"}}"#
            ),
            sourceService: service
        )
        let availabilityValue = try XCTUnwrap(
            decodedResponse(availability)["value"] as? [String: Any]
        )
        XCTAssertEqual(availabilityValue["status"] as? String, "cloud-only")

        let download = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":1,"request_id":"d","command":"icloud-request-download","payload":{"directory_name":"Ayumi","source_id":"2026/10/29/n.md","timeout_milliseconds":5000}}"#
            ),
            sourceService: service
        )
        let downloadValue = try XCTUnwrap(decodedResponse(download)["value"] as? [String: Any])
        XCTAssertEqual(downloadValue["status"] as? String, "resident")

        let read = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":1,"request_id":"r","command":"icloud-coordinated-read","payload":{"directory_name":"Ayumi","source_id":"2026/10/29/n.md","handoff_directory":"/synthetic/private","maximum_bytes":8388608}}"#
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
                #"{"protocol_version":1,"request_id":"u","command":"icloud-availability","payload":{"directory_name":"../Ayumi","source_id":"2026/10/29/n.md"}}"#
            ),
            sourceService: service
        )
        try expectFailure(unsafe, code: "malformed_input", requestID: "u")

        for sourceID in ["2026/13/29/n.md", "2026/02/30/n.md", "２０２６/10/29/n.md"] {
            let invalidDate = BridgeProtocol.handle(
                frame(
                    #"{"protocol_version":1,"request_id":"u","command":"icloud-availability","payload":{"directory_name":"Ayumi","source_id":"\#(sourceID)"}}"#
                ),
                sourceService: service
            )
            try expectFailure(invalidDate, code: "malformed_input", requestID: "u")
        }

        service.error = .permissionDenied
        let denied = BridgeProtocol.handle(
            frame(
                #"{"protocol_version":1,"request_id":"p","command":"icloud-availability","payload":{"directory_name":"Ayumi","source_id":"2026/10/29/n.md"}}"#
            ),
            sourceService: service
        )
        try expectFailure(denied, code: "permission_denied", requestID: "p")
        XCTAssertFalse(String(decoding: denied.response, as: UTF8.self).contains("Ayumi"))
    }

    private func frame(_ json: String) -> Data {
        Data(json.utf8) + Data([0x0A])
    }

    private func decodedResponse(_ outcome: BridgeOutcome) throws -> [String: Any] {
        let object = try JSONSerialization.jsonObject(with: Data(outcome.response.dropLast()))
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
        XCTAssertLessThanOrEqual(response.count, BridgeProtocol.maximumFrameBytes)
        XCTAssertEqual(response.last, 0x0A)
        XCTAssertFalse(response.dropLast().contains(0x0A))
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
