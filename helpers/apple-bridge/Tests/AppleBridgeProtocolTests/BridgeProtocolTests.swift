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
