import AppleBridgeProtocol
import Darwin
import Foundation

@main
struct AppleBridgeCommand {
    static func main() {
        let input = readBoundedFrame()
        let outcome = BridgeProtocol.handle(input)
        FileHandle.standardOutput.write(outcome.response)

        if let diagnosticCode = outcome.diagnosticCode {
            let diagnostic = Data("apple-bridge: \(diagnosticCode)\n".utf8)
            FileHandle.standardError.write(diagnostic)
        }

        if !outcome.succeeded {
            exit(EXIT_FAILURE)
        }
    }

    private static func readBoundedFrame() -> Data {
        var input = Data()

        do {
            while input.count <= BridgeProtocol.maximumFrameBytes {
                let remaining = BridgeProtocol.maximumFrameBytes + 1 - input.count
                guard let chunk = try FileHandle.standardInput.read(upToCount: remaining),
                      !chunk.isEmpty
                else {
                    break
                }
                input.append(chunk)
            }
        } catch {
            return Data()
        }

        return input
    }
}
