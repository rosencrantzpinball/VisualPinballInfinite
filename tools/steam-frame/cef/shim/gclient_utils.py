"""A stand-in for depot_tools' gclient_utils, which CEF's tools/gclient_util.py imports for CheckCallAndFilter and
Error alone: CEF is built here from Chromium's source tarball, without depot_tools (build.sh puts this folder on
PYTHONPATH). Runs the command in its folder, its output shown as it goes, and raises Error when it fails."""
import subprocess


class Error(Exception):
  pass


def CheckCallAndFilter(args, cwd=None, always_show_header=False, print_stdout=False, **_):
  if always_show_header:
    print("________ running '%s' in '%s'" % (' '.join(args), cwd), flush=True)
  result = subprocess.run(args, cwd=cwd, stdout=None if print_stdout else subprocess.PIPE, stderr=subprocess.STDOUT)
  if result.returncode != 0:
    raise Error("Command '%s' returned non-zero exit status %d in %s" % (' '.join(args), result.returncode, cwd))
  return b'' if print_stdout else result.stdout
