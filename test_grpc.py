#!/usr/bin/env python3

import grpc
import sys
import os

# Add the generated proto files to the path
sys.path.insert(0, os.path.dirname(__file__))

# You would need to generate Python gRPC code from the proto file:
# python -m grpc_tools.protoc -I./grpc --python_out=. --grpc_python_out=. ./grpc/race.proto

print("To test gRPC, you need to:")
print("1. Install grpcio-tools: pip install grpcio-tools")
print("2. Generate Python code: python -m grpc_tools.protoc -I./grpc --python_out=. --grpc_python_out=. ./grpc/race.proto")
print("3. Then import and use the generated code")
print()
print("For now, let's test with grpcurl instead...")