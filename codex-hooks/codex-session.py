#!/usr/bin/env python3
"""
Codex Session Monitor - Tracks individual prompts/responses as races
Similar to how Claude hooks work, but for Codex interactive sessions
"""

import sys
import os
import pty
import select
import subprocess
import re
import json
import time
import threading
from datetime import datetime

RACEBOARD_CMD = os.environ.get(
    "RACEBOARD_CMD", 
    "/Users/user/RustroverProjects/RaceboardServer/target/debug/raceboard-codex"
)

class CodexSessionMonitor:
    def __init__(self):
        self.current_race_id = None
        self.in_response = False
        self.prompt_buffer = ""
        self.response_buffer = ""
        
        # Patterns to detect prompts and responses
        # Adjust these based on actual Codex behavior
        self.prompt_patterns = [
            r'^>>> ',           # Python-style prompt
            r'^codex> ',        # Possible Codex prompt
            r'^\? ',            # Question prompt
            r'^> ',             # Simple prompt
            r'Enter.*:$',       # Enter prompt pattern
        ]
        
        self.response_end_patterns = [
            r'^>>> ',           # Next prompt
            r'^codex> ',        # Back to prompt
            r'^\? ',            # Next question
            r'^> ',             # Back to prompt
            r'^Done\.',         # Explicit completion
            r'^Completed',      # Completion message
            r'^\n\n',           # Double newline
        ]
    
    def start_race(self, prompt):
        """Start a new race for a prompt"""
        try:
            # Clean prompt for display
            clean_prompt = prompt.strip()[:50].replace('\n', ' ')
            
            # Start race using raceboard-codex
            result = subprocess.run(
                [RACEBOARD_CMD, "start", "prompt", prompt, 
                 "--title", f"Codex: {clean_prompt}",
                 "--eta", "15"],
                capture_output=True,
                text=True
            )
            
            if result.returncode == 0 and result.stdout:
                self.current_race_id = result.stdout.strip()
                print(f"\r\n🏁 Race started: {self.current_race_id}", file=sys.stderr)
                
                # Start progress updater thread
                threading.Thread(
                    target=self.update_progress,
                    args=(self.current_race_id,),
                    daemon=True
                ).start()
                
                return self.current_race_id
        except Exception as e:
            print(f"Failed to start race: {e}", file=sys.stderr)
        return None
    
    def complete_race(self):
        """Complete the current race"""
        if self.current_race_id:
            try:
                subprocess.run(
                    [RACEBOARD_CMD, "complete", self.current_race_id, 
                     "--exit-code", "0",
                     "--message", f"Response length: {len(self.response_buffer)} chars"],
                    capture_output=True
                )
                print(f"\r\n✅ Race completed: {self.current_race_id}", file=sys.stderr)
            except Exception as e:
                print(f"Failed to complete race: {e}", file=sys.stderr)
            finally:
                self.current_race_id = None
                self.response_buffer = ""
    
    def update_progress(self, race_id):
        """Update progress in background"""
        start_time = time.time()
        eta = 15  # seconds
        
        while self.current_race_id == race_id:
            elapsed = time.time() - start_time
            if elapsed >= eta:
                progress = 95
            else:
                progress = int((elapsed / eta) * 95)
            
            try:
                subprocess.run(
                    [RACEBOARD_CMD, "update", race_id, "--progress", str(progress)],
                    capture_output=True
                )
            except:
                pass
            
            time.sleep(2)
            
            # Safety timeout
            if elapsed > 300:  # 5 minutes
                break
    
    def is_prompt_line(self, line):
        """Check if line matches prompt pattern"""
        for pattern in self.prompt_patterns:
            if re.search(pattern, line):
                return True
        return False
    
    def is_response_end(self, line):
        """Check if line indicates response end"""
        for pattern in self.response_end_patterns:
            if re.search(pattern, line):
                return True
        # Also check for empty line after content
        if self.response_buffer and line.strip() == "":
            return True
        return False
    
    def process_output(self, data):
        """Process output from Codex"""
        # Decode if bytes
        if isinstance(data, bytes):
            text = data.decode('utf-8', errors='ignore')
        else:
            text = data
        
        # Process line by line
        for line in text.split('\n'):
            # Check for prompt
            if self.is_prompt_line(line):
                # If we were in a response, complete it
                if self.in_response:
                    self.complete_race()
                    self.in_response = False
                
                # Mark that we're expecting user input
                self.prompt_buffer = ""
            
            # Check for response end
            elif self.in_response and self.is_response_end(line):
                self.complete_race()
                self.in_response = False
            
            # Accumulate response
            elif self.in_response:
                self.response_buffer += line + '\n'
    
    def process_input(self, data):
        """Process input from user"""
        # Decode if bytes
        if isinstance(data, bytes):
            text = data.decode('utf-8', errors='ignore')
        else:
            text = data
        
        # Check for Enter key (prompt submission)
        if '\r' in text or '\n' in text:
            if self.prompt_buffer.strip():
                # User submitted a prompt
                self.start_race(self.prompt_buffer)
                self.in_response = True
                self.prompt_buffer = ""
        else:
            # Accumulate prompt
            self.prompt_buffer += text
    
    def run(self, command_args):
        """Run Codex with monitoring"""
        # Create a pseudo-terminal
        master_fd, slave_fd = pty.openpty()
        
        # Start Codex process
        process = subprocess.Popen(
            command_args,
            stdin=slave_fd,
            stdout=slave_fd,
            stderr=slave_fd,
            preexec_fn=os.setsid
        )
        
        # Make stdin non-blocking
        import fcntl
        flags = fcntl.fcntl(sys.stdin, fcntl.F_GETFL)
        fcntl.fcntl(sys.stdin, fcntl.F_SETFL, flags | os.O_NONBLOCK)
        
        try:
            while process.poll() is None:
                # Use select to monitor both stdin and master_fd
                ready, _, _ = select.select([sys.stdin, master_fd], [], [], 0.1)
                
                for fd in ready:
                    if fd == sys.stdin:
                        # Read from user input
                        try:
                            data = os.read(sys.stdin.fileno(), 1024)
                            if data:
                                # Process and forward to Codex
                                self.process_input(data)
                                os.write(master_fd, data)
                        except (OSError, IOError):
                            pass
                    
                    elif fd == master_fd:
                        # Read from Codex output
                        try:
                            data = os.read(master_fd, 1024)
                            if data:
                                # Process and forward to terminal
                                self.process_output(data)
                                sys.stdout.buffer.write(data)
                                sys.stdout.flush()
                        except (OSError, IOError):
                            pass
        
        finally:
            # Cleanup
            if self.in_response:
                self.complete_race()
            
            # Restore stdin flags
            fcntl.fcntl(sys.stdin, fcntl.F_SETFL, flags)
            
            # Close PTY
            os.close(master_fd)
            os.close(slave_fd)
            
            # Wait for process
            process.wait()
            
            return process.returncode

def main():
    if len(sys.argv) < 2:
        print("Usage: codex-session.py <codex command and args>", file=sys.stderr)
        sys.exit(1)
    
    # Check if raceboard server is running
    try:
        subprocess.run(
            ["curl", "-s", "http://localhost:7777/health"],
            capture_output=True,
            check=True
        )
    except:
        print("Warning: Raceboard server not running", file=sys.stderr)
    
    # Run Codex with monitoring
    monitor = CodexSessionMonitor()
    exit_code = monitor.run(sys.argv[1:])
    sys.exit(exit_code)

if __name__ == "__main__":
    main()