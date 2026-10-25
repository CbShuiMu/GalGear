"""Native FreeMote synthetic fixtures only; no games, presets or model calls.

The embedded v2/v3 files were produced by bundled PsBuild v4.9.0 plus
PsbResource/Merge/Build. Encryption uses two explicitly self-chosen u32 keys.
JSON/native Build creates test inputs only; decryption never rebuilds a PSB.
"""
import base64
import copy
import hashlib
import os
import pathlib
import runpy
import struct
import tempfile
import unittest
import zlib
from unittest import mock

BASE = pathlib.Path(__file__).parent
CODECS = runpy.run_path(str(BASE/'bytecodecs.py'))
TYPED = runpy.run_path(str(BASE/'typed_evidence.py'))
EVIDENCE = runpy.run_path(str(BASE/'evidence.py'))
ARCHIVES = runpy.run_path(str(BASE/'archive_evidence.py'))
HOST = runpy.run_path(str(BASE/'agent_host.py'))

KEYS={2:0x13579bdf,3:0x2468ace1}
RAW={
    2:base64.b64decode('UFNCAAIAAAAoAAAAKAAAAPkBAAAEAgAAggIAAIYCAACQAgAAnQEAAA6vAA0BAAECAwQFBgcICQoLAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACAAQBAQ8BAwICAQEFDQgBDhQDJS01AT8GBAEKAQoKDgQDEREcEh4ZGwQoHRsqHAUiIi4GNCszCCcHIygJLys0PAo+CzMzPjk7DAAAAAAyDq8ADQBmaXKJj5OZl5yho6kAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHoAbwCBZHwAYIBjaGduAGplAIIAAABiAHlrdnh7bH1+f21wcYOEhYaHiHOKi4yNjnSQkZKdlJWUlph1mpt0dZ6foK6id6SlpqeoAAAAAHUNDA0BAgMEBQYHCAkKCwwhDQYNAAIEBwkKDQYNADsIAgYEGQAVBxUAFQEgDQINABchDQQNAwUGCA0EDQAGAgQFBxUEFQMVAiENBA0DBQYIDQQNAQMABQEFCxUFFQYhDQINAQsNAg0AAQMFAg0IDXBJZBw3WAB6RGlmZmVyZW50IHN5bnRoZXRpYyByZWNvcmQuAEluZGVwZW5kZW50IGZpcnN0IGNhcHRpb24uAFN5bnRoZXRpYyBzcGVha2VyAFN5bnRoZXRpY0FscGhhAGNoYXB0ZXJfdHdvAGNoYXB0ZXJfb25lAHN5bnRoZXRpYwB3aW4ADQENAA0BDgABAAAAAAAAAQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyAhIiMkJSYnKCkqKywtLi8wMTIzNDU2Nzg5Ojs8PT4/QEFCQ0RFRkdISUpLTE1OT1BRUlNUVVZXWFlaW1xdXl9gYWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXp7fH1+f4CBgoOEhYaHiImKi4yNjo+QkZKTlJWWl5iZmpucnZ6foKGio6SlpqeoqaqrrK2ur7CxsrO0tba3uLm6u7y9vr/AwcLDxMXGx8jJysvMzc7P0NHS09TV1tfY2drb3N3e3+Dh4uPk5ebn6Onq6+zt7u/w8fLz9PX29/j5+vv8/f7/'),
    3:base64.b64decode('UFNCAAMAAAAsAAAALAAAAP0BAAAIAgAAkQIAAJUCAACgAgAAoQEAAM8D2joOrwANAQABAgMEBQYHCAkKCwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAEAQEPAQMCAgEBBQ0IAQ4UAyUtNQE/BgQBCgEKCg4EAxERHBIeGRsEKB0bKhwFIiIuBjQrMwgnByMoCS8rNDwKPgszMz45OwwAAAAAMg6vAA0AZmlyiY+TmZecoaOpAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB6AG8AgWR8AGCAY2hnbgBqZQCCAAAAYgB5a3Z4e2x9fn9tcHGDhIWGh4hziouMjY50kJGSnZSVlJaYdZqbdHWen6CuonekpaanqAAAAAB1DQwNAQIDBAUGBwgJCgsMIQ0GDQACBAcJCg0GDQA7CAIGBBkAFQcVABUBIA0CDQAXIQ0EDQMFBggNBA0ABgIEBQcVBBUDFQIhDQQNAwUGCA0EDQEDAAUBBQsVBRUGIQ0CDQELDQINAAEDBQMNCA17VW8AQ2MnhUFub3RoZXIgaW5kZXBlbmRlbnQgc3ludGhldGljIGNhcHRpb24uAERpZmZlcmVudCBzeW50aGV0aWMgcmVjb3JkLgBTeW50aGV0aWMgc3BlYWtlcgBTeW50aGV0aWNCZXRhAGNoYXB0ZXJfdHdvAGNoYXB0ZXJfb25lAHN5bnRoZXRpYwB3aW4ADQENAA0BDgABAAAAAAAAAAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+P0BBQkNERUZHSElKS0xNTk9QUVJTVFVWV1hZWltcXV5fYGFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6e3x9fn+AgYKDhIWGh4iJiouMjY6PkJGSk5SVlpeYmZqbnJ2en6ChoqOkpaanqKmqq6ytrq+wsbKztLW2t7i5uru8vb6/wMHCw8TFxsfIycrLzM3Oz9DR0tPU1dbX2Nna29zd3t/g4eLj5OXm5+jp6uvs7e7v8PHy8/T19vf4+fr7/P3+/w=='),
}
CIPHERS={
    (2,'Header'):base64.b64decode('UFNCAAIAAQDtz73KIp5PDXzCDoNcOygsfRrPCKRqAXn8nOaMzJVp4Q6vAA0BAAECAwQFBgcICQoLAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACAAQBAQ8BAwICAQEFDQgBDhQDJS01AT8GBAEKAQoKDgQDEREcEh4ZGwQoHRsqHAUiIi4GNCszCCcHIygJLys0PAo+CzMzPjk7DAAAAAAyDq8ADQBmaXKJj5OZl5yho6kAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHoAbwCBZHwAYIBjaGduAGplAIIAAABiAHlrdnh7bH1+f21wcYOEhYaHiHOKi4yNjnSQkZKdlJWUlph1mpt0dZ6foK6id6SlpqeoAAAAAHUNDA0BAgMEBQYHCAkKCwwhDQYNAAIEBwkKDQYNADsIAgYEGQAVBxUAFQEgDQINABchDQQNAwUGCA0EDQAGAgQFBxUEFQMVAiENBA0DBQYIDQQNAQMABQEFCxUFFQYhDQINAQsNAg0AAQMFAg0IDXBJZBw3WAB6RGlmZmVyZW50IHN5bnRoZXRpYyByZWNvcmQuAEluZGVwZW5kZW50IGZpcnN0IGNhcHRpb24uAFN5bnRoZXRpYyBzcGVha2VyAFN5bnRoZXRpY0FscGhhAGNoYXB0ZXJfdHdvAGNoYXB0ZXJfb25lAHN5bnRoZXRpYwB3aW4ADQENAA0BDgABAAAAAAAAAQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyAhIiMkJSYnKCkqKywtLi8wMTIzNDU2Nzg5Ojs8PT4/QEFCQ0RFRkdISUpLTE1OT1BRUlNUVVZXWFlaW1xdXl9gYWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXp7fH1+f4CBgoOEhYaHiImKi4yNjo+QkZKTlJWWl5iZmpucnZ6foKGio6SlpqeoqaqrrK2ur7CxsrO0tba3uLm6u7y9vr/AwcLDxMXGx8jJysvMzc7P0NHS09TV1tfY2drb3N3e3+Dh4uPk5ebn6Onq6+zt7u/w8fLz9PX29/j5+vv8/f7/'),
    (2,'Body'):base64.b64decode('UFNCAAIAAAAoAAAAKAAAAPkBAAAEAgAAggIAAIYCAACQAgAAnQEAAMtgvccLnk4PhscLhV8xISb0GM8IImgBeWye5oxRlGnhY2ARkRtKI+PtvY5b0WHo9hcV4OwC6WoVl18lO8tXdo6/o9JiHnSyIHhOejGTj44Nxhq2+gqTF0neHf2qBe1204e9XJln4iJt0x52Ld7NDEua/sk35h6sSCNKwcSDOJbV4nv56JvcD+Lh5MASDJTjcD+i1hWnbQMT9xGL4exD4fBWUyaoAFMd6Io6W/I55EsB8INh+BDOTQ9JT/lly1tHJXAwW3F6icTRM3fLZAYPEyAZjXg8tVwCyUnYT/a5kpxONs50trnLSG3J203lg5TuTzbm+19z+8V0sfXd/0UsrMSwqOZEPN2jHwTTww+9GKiqcIZ62x7qMaCvNntK6aH8QUT10kl56OQe6PIpapW6/tQzAXAskp60JDIGNPl2tqKmsUEIvuh/1U0OHW/1RAzS/oMI69dXG8E1KjDgWS+vMNXPOMi1iwO66FQTOd1DnD0br5jO4xaAlbFnxDW1rYsMfIJhg4ZXmGfLVANvxy/aOYR3bG8uB6M87SaubvUZQn4X8vZaWRvJ60NXFAbenKiiRjdn/fjGzkDDzU+oKJ/WfgZ7qZcP3FIux/6ngKR4dNUR0kQPuklrGneWoBetykOrFuvWbJOIHo8ePGQuahq7pTaeTDjJd4z4IITGSXqUOTZbz2hHYA8xQKT2yUjEz0oANyBLt9NY+netn2b3XFA8Y0uIqdBn4q23ojdEbXf06JXbdN2sDPILLsd7ThWqKamT5nBamffi/xUUyZHxh0yu9PMdYMConIQH3zY0DQENAA0BDgABAAAAAAAAAQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyAhIiMkJSYnKCkqKywtLi8wMTIzNDU2Nzg5Ojs8PT4/QEFCQ0RFRkdISUpLTE1OT1BRUlNUVVZXWFlaW1xdXl9gYWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXp7fH1+f4CBgoOEhYaHiImKi4yNjo+QkZKTlJWWl5iZmpucnZ6foKGio6SlpqeoqaqrrK2ur7CxsrO0tba3uLm6u7y9vr/AwcLDxMXGx8jJysvMzc7P0NHS09TV1tfY2drb3N3e3+Dh4uPk5ebn6Onq6+zt7u/w8fLz9PX29/j5+vv8/f7/'),
    (2,'Full'):base64.b64decode('UFNCAAIAAQDtz73KIp5PDXzCDoNcOygsfRrPCKRqAXn8nOaMzJVp4W3PEZwaSiLh7rmLXdZp4fwcFeDsAulqFZdfJTvLV3aOv6PSYh50siB4Tnoxk4+ODcYatvoKkxdJ3h39qgXtdtOHvVyZZeImbNIRdy7czw1Kn/PBNugKr20Of8D7hTyX3+Nx8+ad3xry/PnfCBWSymwhhcIRi1suMO4PudbNQMPSXnYHkjhaMvKlG3vSGexjHOupT/Od7G4JGw245GPP/ZrlvtPsRoP62gBE9V09AxMgGY1KMhpcD8kvsT1/NgEF2apv1x+5y0htydtN5YOU7k825vtfc/vFdLH13f9FLKzEsKjmRDzdox8E08MPvRioqnCGetse6jHar1l7y43d/CHElrouF+iOe+hwKWqV2P6tWHcIV/7jyiFfGUX7lk8kQblR6lIK8jFcng79aND7RhFwCwk3TxMh1efuM60PjxD170tCPgf7OZDJg6lD0wyvij/kXnNu1AYoymqVxg4noNYsbIO9X+9sw0ACeMM+3yqCX2hnKAuVEOYvo2/0GEN5Hvn7XGAXzupSSgEQzIuJukMaafrzztRlw8hBqyiY23gXc7yXLNVVJNPxv4G8elTbEN1KArExL3pmoPsX0o8vxmWLsQ/c8TzxZlkdRAJu08UT7iRTq3Whsjz68C1loDU+Wc90Vi4deEGu7J1DwMtXCng8ANTvU/AtxbN8+loAKn1KjKzBNYSXvL83DGti7f+92GqbzV/oDTvfal8Oln2tjINyWZ31lskeJdKX8fNWtNvrBW3VwZzsEcYsUUvJ+10kx91o1NafWqLLsixxyxFv+yH2OV8yFB9mJ6y+DQENAA0BDgABAAAAAAAAAQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyAhIiMkJSYnKCkqKywtLi8wMTIzNDU2Nzg5Ojs8PT4/QEFCQ0RFRkdISUpLTE1OT1BRUlNUVVZXWFlaW1xdXl9gYWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXp7fH1+f4CBgoOEhYaHiImKi4yNjo+QkZKTlJWWl5iZmpucnZ6foKGio6SlpqeoqaqrrK2ur7CxsrO0tba3uLm6u7y9vr/AwcLDxMXGx8jJysvMzc7P0NHS09TV1tfY2drb3N3e3+Dh4uPk5ebn6Onq6+zt7u/w8fLz9PX29/j5+vv8/f7/'),
    (3,'Header'):base64.b64decode('UFNCAAMAAQAw/oL9GKlwOqHzMbSXTV/VpzvHP2YMQYAVrdm7Ry3VlAy6B6gOrwANAQABAgMEBQYHCAkKCwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAEAQEPAQMCAgEBBQ0IAQ4UAyUtNQE/BgQBCgEKCg4EAxERHBIeGRsEKB0bKhwFIiIuBjQrMwgnByMoCS8rNDwKPgszMz45OwwAAAAAMg6vAA0AZmlyiY+TmZecoaOpAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB6AG8AgWR8AGCAY2hnbgBqZQCCAAAAYgB5a3Z4e2x9fn9tcHGDhIWGh4hziouMjY50kJGSnZSVlJaYdZqbdHWen6CuonekpaanqAAAAAB1DQwNAQIDBAUGBwgJCgsMIQ0GDQACBAcJCg0GDQA7CAIGBBkAFQcVABUBIA0CDQAXIQ0EDQMFBggNBA0ABgIEBQcVBBUDFQIhDQQNAwUGCA0EDQEDAAUBBQsVBRUGIQ0CDQELDQINAAEDBQMNCA17VW8AQ2MnhUFub3RoZXIgaW5kZXBlbmRlbnQgc3ludGhldGljIGNhcHRpb24uAERpZmZlcmVudCBzeW50aGV0aWMgcmVjb3JkLgBTeW50aGV0aWMgc3BlYWtlcgBTeW50aGV0aWNCZXRhAGNoYXB0ZXJfdHdvAGNoYXB0ZXJfb25lAHN5bnRoZXRpYwB3aW4ADQENAA0BDgABAAAAAAAAAAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+P0BBQkNERUZHSElKS0xNTk9QUVJTVFVWV1hZWltcXV5fYGFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6e3x9fn+AgYKDhIWGh4iJiouMjY6PkJGSk5SVlpeYmZqbnJ2en6ChoqOkpaanqKmqq6ytrq+wsbKztLW2t7i5uru8vb6/wMHCw8TFxsfIycrLzM3Oz9DR0tPU1dbX2Nna29zd3t/g4eLj5OXm5+jp6uvs7e7v8PHy8/T19vf4+fr7/P3+/w=='),
    (3,'Body'):base64.b64decode('UFNCAAMAAAAsAAAALAAAAP0BAAAIAgAAkQIAAJUCAACgAgAAoQEAAM8D2joSUYLwNalxOF/2NLKYR1bfPTnHP/MOQYC1r9m75izVlMO53ZL9C2MaA6uxbG8IPlEqsFIu/mpsLMM1ZM1wIbhsLR/Q17OkpJhJUIJ0CEyc2WEJ4o4ypd4zGLu4VatBf24E5EjwCDh9N6cqiKXHWG4zTf24hJIl9V9curyqd1JN1yHFdtGGfNwFca5sdHeUGs/eahLuayFukkx/GmRGH/0CnlJkh8BwvyzTUxM4j2Hq25sQsx0i3vY/lPrJ3LNALUpFpAlvN6REJy6H6a78jL/VX9+tQwGuykHhgfijDiEJijdAc6d+SN2zbSBN1LtqjxeF4sAqG8EHc17J9M6JuEKijrK7j3u1+cJDKi+qf0keHZCnY09qkT46HLmQEsRcKhwSNE9JkyZM/5OhE4/xXxHbjQPmzWrBWmLne0yILcRup59CFu6i+O4pw9VwI7sq8UqlTG/DfIv8metMrN1GCcX8ykquqoEGILcYw+5lknOoJ7PcgfNcIecuP5MULGdDVXa7Y9/m3h9JoVeFST5axMS5TvJfk973AE4FZombJT9CrMYLntsyIvSA7pHvTfl3xo1X0RGW8ZDWkVySJaUtcoFmAOg8ZAOfAkyATy7MlX885GFod36VNMZCaSgVZA4kjI9MEU1T1aP0H6/X5m32XqzIi56gFBheu/UfXP6xJOaBlth1DqrVj5luUW/URHI3BOPUBVdlWrdUl8lSEHfhlEcxeeruiAl1ZcDQxXI6Zter6IITTXGW7JJdFhj/YLZByKq1JYiwKXJ3rouLTopHe/DCZRwBvG9HdNJYJ2nuRQklbPJPC2xLDQENAA0BDgABAAAAAAAAAAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+P0BBQkNERUZHSElKS0xNTk9QUVJTVFVWV1hZWltcXV5fYGFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6e3x9fn+AgYKDhIWGh4iJiouMjY6PkJGSk5SVlpeYmZqbnJ2en6ChoqOkpaanqKmqq6ytrq+wsbKztLW2t7i5uru8vb6/wMHCw8TFxsfIycrLzM3Oz9DR0tPU1dbX2Nna29zd3t/g4eLj5OXm5+jp6uvs7e7v8PHy8/T19vf4+fr7/P3+/w=='),
    (3,'Full'):base64.b64decode('UFNCAAMAAQAw/oL9GKlwOqHzMbSXTV/VpzvHP2YMQYAVrdm7Ry3VlAy6B6jzpGMXAquwbmwMO1ctuFsk9WpsLMM1ZM1wIbhsLR/Q17OkpJhJUIJ0CEyc2WEJ4o4ypd4zGLu4VatBf24E5EjwCjh5NqYliabFWm8ySPCwhZwx9npxj72VcVZM3SDPfN+Cf80Ub7x2bG2fM9HHQg/qTA5IlXZAKklMLd8VkXlOuf1wiynkYzwQqH/0woAUqQ6W9Oc60LGVUwj4h9X+AonuPotvExKN16XPv4HsZNOtQwGu+E9OgfWjaEh7A7jT6jDi6X4abSBN1LtqjxeF4sAqG8EHc17J9M6JuEKijrK7j3u1+cJDKi+qf0keHZCnY09qkT5AHNaQk6AgKnySVycu/SYmmpMjE4/xPRGi5nWetga8JB2KCz1xqS7ooXNNnAWuFgg6PUSI2y89Zdw9W/UhY4gafSefcNWP3BLY5s8oLQkApzCZT2ISBuc8vS9AH27IdXCzRuaIt8Dk/QwZy3l6fh1PpU7wUTVCxdO7avpbmdbpK0gNSoeYLjdNrMwCktQwKvOu4obqXOJW3o5P0gGR2ZDQkV2GBK0oeol2A/U3QQOdCU+PRyvUkWs89GBEcnfjYqxEIkY27E5J4/4lcTRmudiWW9KwhQiYPdrl+ObNZXM2x5EHKfLQF/HPfPc1YZrUjI0rSmTeVSIhE+nFA0YxQK0akcRUC2zmmiQDcPDzjwIvDOeZ0GQ6ddmg7qIzTXGW7JJdFhidd6dDp7u5aviXNW5cspmQJ4oPafDTYQU7oQFxDc9VIXjySR4OaoZZYmEjEXYwQxIN6jLT948R0CKXZW+JDSB5hZkC9+Rka0ogswVtHmE6DQENAA0BDgABAAAAAAAAAAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+P0BBQkNERUZHSElKS0xNTk9QUVJTVFVWV1hZWltcXV5fYGFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6e3x9fn+AgYKDhIWGh4iJiouMjY6PkJGSk5SVlpeYmZqbnJ2en6ChoqOkpaanqKmqq6ytrq+wsbKztLW2t7i5uru8vb6/wMHCw8TFxsfIycrLzM3Oz9DR0tPU1dbX2Nna29zd3t/g4eLj5OXm5+jp6uvs7e7v8PHy8/T19vf4+fr7/P3+/w=='),
}


HISTORICAL_SAVE=base64.b64decode('UFNCAAMAAAAsAAAALAAAAGoBAABvAQAApAEAAKcBAACwAQAATQEAAIADIjYOiQANAQMAAgEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABgACAQECCwMEAgEDAgEOAQEQAQYXAQMEAwwFCQYKAg0YDhoVFwMSFgQOiQANAG9+hWgAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAbQBkZQBseohnAHVpeGN8cGoAYncAcmt2c2B5ZntufXF/gIGCg4R0hocNBA0CBAMBIQ0BDQANAQ0AIQ0DDQECAw0DDQACBRUABtUBFQENAg0nAEFub3RoZXIgaW5kZXBlbmRlbnQgc3ludGhldGljIGNhcHRpb24uAHN5bnRoZXRpYy5wc2IADQAMDQAMAAAAAAAA')


def mdf(data):
    return b'mdf\0'+len(data).to_bytes(4,'little')+zlib.compress(data)


def schema(version, *, frame=False):
    result=dict(syntax='psb',view=[],crypto=dict(codec='psb_decrypt',key=KEYS[version]))
    if frame:
        result['frame']=dict(codec='mdf')
    return result


def field(document,path,kind='string'):
    return dict(kind='typed_field',document=copy.deepcopy(document),path=path,type=kind)


def rows_for(data,document):
    rows=[]
    for index in range(2):
        record=['records',index]
        proof=field(document,record+['text'])
        selected=TYPED['replay_field'](data,proof)
        root=TYPED['parse_document'](data,document)['root']
        speaker=TYPED['select'](root,record+['speaker'])
        shown=speaker['value'] if speaker['type']=='string' else None
        if shown:
            proof['speaker']=field(document,record+['speaker'])
        proof['native']=dict(kind='typed_record',record=record,scope=record+['scope'],
            coordinate=record+['position'],text=record+['text'],speaker=record+['speaker'])
        rows.append(dict(position=selected['span']['offset'],text=selected['value'],speaker=shown,evidence=proof))
    return rows


def xp3(payload, checksum):
    def chunk(tag,data):
        return tag+struct.pack('<Q',len(data))+data
    name='internal.psb'.encode('utf-16-le')
    info=struct.pack('<IQQH',0,len(payload),len(payload),len(name)//2)+name
    segments=struct.pack('<IQQQ',0,19,len(payload),len(payload))
    index=chunk(b'File',chunk(b'info',info)+chunk(b'segm',segments)+chunk(b'adlr',struct.pack('<I',checksum)))
    return ARCHIVES['MAGIC']+struct.pack('<Q',19+len(payload))+payload+b'\0'+struct.pack('<Q',len(index))+index


class MdfFrameTests(unittest.TestCase):
    def test_psb_metadata_names_and_ids_share_one_decoded_document_not_one_descriptor(self):
        data=mdf(RAW[3])
        document=dict(syntax='psb',view=[],frame=dict(codec='mdf'))
        name=field(document,['records',0,'speaker'])
        token=field(dict(syntax='psb',frame=dict(codec='mdf')),['records',0,'position'],'integer')
        shown=TYPED['replay_field'](data,name)['value']
        native=TYPED['replay_field'](data,token)['value']
        output=dict(values={},speakers=dict(one=dict(id=native,text=shown,evidence=name,id_evidence=token)))
        self.assertEqual(len(EVIDENCE['speaker_metadata'](data,output)[1]),1)
        replay=EVIDENCE['TYPED']['replay_field']
        def other_document(raw,proof):
            selected=replay(raw,proof)
            if proof['path']==['records',0,'position']:
                selected['document_sha256']=hashlib.sha256(RAW[2]).hexdigest()
            return selected
        with mock.patch.dict(EVIDENCE['TYPED'],replay_field=other_document):
            with self.assertRaisesRegex(ValueError,'one complete decoded document'):
                EVIDENCE['speaker_metadata'](data,output)

    def test_complete_frame_replays_whole_binary_and_rejects_non_native_parameters(self):
        for raw in RAW.values():
            wrapped=mdf(raw)
            self.assertEqual(CODECS['decode']('mdf',wrapped,len(raw)),raw)
            self.assertEqual(CODECS['decode']('mdf',bytearray(wrapped)),raw)
            self.assertEqual(CODECS['view'](wrapped,[dict(op='mdf')]),raw)
            for changed in (wrapped[:-1],wrapped+b'extra',b'unknown'+wrapped,
                            wrapped[:4]+(len(raw)+1).to_bytes(4,'little')+wrapped[8:],
                            wrapped[:4]+(CODECS['MAX_BYTES']+1).to_bytes(4,'little')+wrapped[8:]):
                with self.subTest(size=len(changed)),self.assertRaises(ValueError):
                    CODECS['decode']('mdf',changed)
        for extra in (dict(offset=8),dict(key=7),dict(seed='synthetic'),dict(path='external'),dict(mode=0)):
            with self.assertRaises(ValueError):
                CODECS['view'](mdf(b'test'),[dict(op='mdf',**extra)])
        with self.assertRaises(ValueError):
            CODECS['decode']('mdf',mdf(b'test'),key=3)

    def test_psb_document_frame_preserves_tree_and_native_reference_positions(self):
        for raw in RAW.values():
            expected=TYPED['parse_document'](raw,dict(syntax='psb',view=[]))
            restored=TYPED['parse_document'](mdf(raw),dict(syntax='psb',view=[],frame=dict(codec='mdf')))
            self.assertEqual(expected['root'],restored['root'])
            self.assertEqual(expected['sha256'],restored['sha256'])
            with self.assertRaises(ValueError):
                TYPED['parse_document'](raw,dict(syntax='psb',view=[dict(op='slice',offset=0,length=len(raw))]))
            for invalid in (None,False,0,{},''):
                with self.subTest(view_type=type(invalid).__name__),self.assertRaises(ValueError):
                    TYPED['parse_document'](raw,dict(syntax='psb',view=invalid))

    def test_mdf_depth_budget_cannot_downgrade_into_output_and_evidence_pool_views(self):
        original=RAW[3]
        selected=TYPED['replay_field'](original,field(dict(syntax='psb',view=[]),['records',0,'text']))
        data=original
        for _ in range(9):
            data=mdf(data)
        pool=dict(selected['node']['string_span'],view=[dict(op='mdf')])
        row=dict(position=0,text=selected['value'],speaker=None,evidence=pool)
        self.assertTrue(EVIDENCE['_looks_like_psb'](data))
        with self.assertRaisesRegex(ValueError,'PSB scripts require'):
            EVIDENCE['script_rows'](data,dict(view=[dict(op='mdf')]*8,rows=[row]))
        name=TYPED['replay_field'](original,field(dict(syntax='psb',view=[]),['records',0,'speaker']))
        pointer=TYPED['replay_field'](original,field(dict(syntax='psb',view=[]),['records',0,'position'],'integer'))
        metadata=dict(values={},view=[dict(op='mdf')]*8,speakers=dict(one=dict(id=pointer['value'],text=name['value'],
            evidence=dict(name['node']['string_span'],view=[dict(op='mdf')]),
            id_evidence=dict(offset=pointer['span']['offset']+1,width=pointer['span']['length']-1,view=[dict(op='mdf')]))))
        with self.assertRaisesRegex(ValueError,'complete typed documents'):
            EVIDENCE['speaker_metadata'](data,metadata)
        facts=dict(source='synthetic.psb',line=0,anchors=[selected['value']],evidence=dict(anchor=pool,source=pool,position=dict(offset=0,width=4)))
        with self.assertRaisesRegex(ValueError,'PSB current-node'):
            EVIDENCE['save_facts'](data,facts,[])


@unittest.skipUnless(os.name=='nt','The fixed bundled native API requires Windows')
class NativePsbCryptoTests(unittest.TestCase):
    def test_valid_plaintext_with_zero_width_arrays_never_reaches_native_auto(self):
        empty=b'\x0d\0\x0c'
        # A complete standard v2 root with zero-width empty packed arrays.
        # Native's first-array heuristic can call this plaintext encrypted.
        raw=b'PSB\0'+struct.pack('<HH8I',2,0,40,40,49,52,56,59,62,52)+empty*4+b'\x20'+empty+empty*2
        self.assertEqual(len(raw),62)
        self.assertEqual(TYPED['parse_document'](raw,dict(syntax='psb',view=[]))['root']['type'],'array')
        with mock.patch.object(CODECS['subprocess'],'run',side_effect=AssertionError('Plaintext must never invoke native Auto')):
            with self.assertRaisesRegex(ValueError,'already complete plaintext'):
                CODECS['decode']('psb_decrypt',raw,key=KEYS[2])

    def test_native_auto_preserves_every_byte_raw_resource_and_typed_reference(self):
        for (version,position),encrypted in CIPHERS.items():
            with self.subTest(version=version,position=position):
                decoded=CODECS['decode']('psb_decrypt',encrypted,key=KEYS[version])
                self.assertEqual(decoded,RAW[version])
                self.assertEqual(CODECS['view'](encrypted,[dict(op='psb_decrypt',key=KEYS[version])]),decoded)
                expected=TYPED['parse_document'](RAW[version],dict(syntax='psb',view=[]))['root']
                actual=TYPED['parse_document'](encrypted,schema(version))['root']
                self.assertEqual(actual,expected)
                resource=actual['value']['binary_fixture']['value']
                self.assertEqual(resource['length'],256)
                self.assertEqual(resource['sha256'],hashlib.sha256(bytes(range(256))).hexdigest())
                proof=field(schema(version),['records',0,'text'])
                selected=TYPED['replay_field'](encrypted,proof)
                original=TYPED['replay_field'](RAW[version],field(dict(syntax='psb',view=[]),['records',0,'text']))
                self.assertEqual(selected['span'],original['span'])
                self.assertEqual(EVIDENCE['span'](encrypted,proof),original['value'])

    def test_plaintext_wrong_key_unknown_fields_and_unbounded_arguments_are_rejected(self):
        for version,raw in RAW.items():
            with self.assertRaises(ValueError):
                CODECS['decode']('psb_decrypt',raw,key=KEYS[version])
            with self.assertRaises(ValueError):
                CODECS['decode']('psb_decrypt',CIPHERS[version,'Full'],key=KEYS[version]+1)
        encrypted=CIPHERS[3,'Full']
        for key in (None,True,-1,2**32,'123',1.5,{},[]):
            with self.subTest(key_type=type(key).__name__),self.assertRaises(ValueError):
                CODECS['decode']('psb_decrypt',encrypted,key=key)
        for extra in (dict(path='external'),dict(assembly='external'),dict(position='Full'),dict(mode='Header'),dict(offset=8)):
            with self.assertRaises(ValueError):
                CODECS['view'](encrypted,[dict(op='psb_decrypt',key=KEYS[3],**extra)])
        with self.assertRaises(TypeError):
            CODECS['decode']('psb_decrypt',encrypted,key=KEYS[3],path='external')
        with self.assertRaises(ValueError):
            CODECS['decode']('psb_decrypt',b'PSB\0\4\0'+encrypted[6:],key=KEYS[3])
        for flags in (2,0xffff):
            with self.assertRaises(ValueError):
                CODECS['decode']('psb_decrypt',encrypted[:6]+flags.to_bytes(2,'little')+encrypted[8:],key=KEYS[3])

    def test_frame_then_crypto_replays_one_complete_binary_not_a_pool(self):
        for version in (2,3):
            encrypted=CIPHERS[version,'Full']
            wrapped=mdf(encrypted)
            document=schema(version,frame=True)
            tree=TYPED['parse_document'](wrapped,document)
            self.assertEqual(tree['root'],TYPED['parse_document'](RAW[version],dict(syntax='psb',view=[]))['root'])
            rows=rows_for(wrapped,document)
            self.assertEqual(len(EVIDENCE['script_rows'](wrapped,dict(rows=rows))),2)
            with self.assertRaisesRegex(ValueError,'PSB scripts require'):
                EVIDENCE['script_rows'](wrapped,dict(view=[dict(op='mdf'),dict(op='psb_decrypt',key=KEYS[version]),
                    dict(op='slice',offset=0,length=20)],rows=rows))
            for change in (dict(crypto=dict(codec='psb_decrypt',key=KEYS[version],offset=0)),
                           dict(frame=dict(codec='mdf',seed='external')),
                           dict(frame=dict(codec='unknown')),dict(view=[dict(op='mdf')])):
                with self.assertRaises(ValueError):
                    TYPED['parse_document'](wrapped,dict(document,**change))

    def test_xp3_receipt_stays_at_the_original_internal_ciphertext_boundary(self):
        version=3
        encrypted=CIPHERS[version,'Full']
        clear=RAW[version]
        with tempfile.TemporaryDirectory(prefix='synthetic-layer-') as folder:
            root=pathlib.Path(folder)
            (root/'game.exe').write_bytes(b'Synthetic executable, never run')
            archive=root/'native.xp3'
            archive.write_bytes(xp3(encrypted,zlib.adler32(encrypted)&0xffffffff))
            receipt=ARCHIVES['verify_member'](archive,'internal.psb',encrypted,original=encrypted)
            self.assertTrue(receipt['verified'])
            with self.assertRaisesRegex(ValueError,'whole restored member'):
                ARCHIVES['verify_member'](archive,'internal.psb',clear,original=encrypted)
            # Internal crypto independently proves the complete typed tree.
            rows=rows_for(encrypted,schema(version))
            self.assertEqual(len(EVIDENCE['script_rows'](encrypted,dict(rows=rows))),2)
            agent=HOST['Agent'](dict(game=str(root),save=str(root),skill=str(root),work=str(root/'work'),language='en'))
            extracted=agent.work/'unpacked';extracted.mkdir()
            member=extracted/'internal.psb';member.write_bytes(encrypted)
            agent.extracted_from[str(extracted)]=str(archive)
            with self.assertRaisesRegex(ValueError,'whole restored member'):
                agent.verify_member_filter(member,encrypted,dict(member_filter=[dict(op='psb_decrypt',key=KEYS[version])]))
            # A different synthetic native producer may declare the clear checksum.
            archive.write_bytes(xp3(encrypted,zlib.adler32(clear)&0xffffffff))
            # Refresh through the real host extraction/stamp path. Only the CLI
            # transport is a synthetic fixture; no provenance guard is relaxed.
            agent.extracted_from.clear()
            agent.extraction_stamps.clear()
            agent.read_skill=True
            def extract(command,**kwargs):
                target=pathlib.Path(command[command.index('-o')+1])
                (target/'internal.psb').write_bytes(encrypted)
                return HOST['subprocess'].CompletedProcess(command,0,b'',b'')
            with mock.patch.object(HOST['subprocess'],'run',side_effect=extract):
                extracted_result=agent.dispatch('run_skill',dict(command='x',archive=archive.name,filter='internal.psb'))
            member=agent.work/extracted_result['output']/'internal.psb'
            self.assertEqual(agent.extraction_stamps[str(member.parent)],agent.file_fingerprint(archive))
            restored,verified=agent.verify_member_filter(member,encrypted,dict(member_filter=[dict(op='psb_decrypt',key=KEYS[version])]))
            self.assertEqual(restored,clear)
            self.assertTrue(verified['verified'])

    def test_all_rows_use_one_effective_binary_even_when_descriptors_differ(self):
        encrypted=CIPHERS[3,'Full']
        rows=rows_for(encrypted,schema(3))
        rows[1]['evidence']['document'].pop('view')
        self.assertEqual(len(EVIDENCE['script_rows'](encrypted,dict(rows=rows))),2)
        replay=EVIDENCE['TYPED']['replay_field']
        def other_document(data,proof):
            selected=replay(data,proof)
            if proof['path']==['records',1,'text']:
                # Isolate aggregation policy with two valid native tree identities;
                # the actual native crypto decoder is tested independently above.
                selected['document_sha256']=hashlib.sha256(RAW[2]).hexdigest()
            return selected
        with mock.patch.dict(EVIDENCE['TYPED'],replay_field=other_document):
            with self.assertRaisesRegex(ValueError,'one complete decoded document'):
                EVIDENCE['script_rows'](encrypted,dict(rows=rows))

    def test_formal_program_rejects_moving_required_outer_filter_into_document_crypto(self):
        program='''import galgear_codecs
import galgear_structured
def extract(data,metadata):
    settings=metadata["resources"]["synthetic"]
    document={"syntax":"psb","view":[]}
    steps=[]
    recovered=data
    if settings["outer_filter"]:
        recovered=galgear_codecs.decode("psb_decrypt",data,key=settings["key"])
        steps=[{"op":"psb_decrypt","key":settings["key"]}]
    else:
        document["crypto"]={"codec":"psb_decrypt","key":settings["key"]}
        if settings["frame"]:
            document["frame"]={"codec":"mdf"}
    root=galgear_structured.parse_document(recovered,document)["root"]
    rows=[]
    for index in range(2):
        record=["records",index]
        proof={"kind":"typed_field","document":document,"path":record+["text"],"type":"string"}
        text=galgear_structured.replay_field(recovered,proof)
        speaker=root["value"]["records"]["value"][index]["value"]["speaker"]
        shown=speaker["value"] if speaker["type"]=="string" else None
        if shown:
            proof["speaker"]={"kind":"typed_field","document":document,"path":record+["speaker"],"type":"string"}
        proof["native"]={"kind":"typed_record","record":record,"scope":record+["scope"],"coordinate":record+["position"],"text":record+["text"],"speaker":record+["speaker"]}
        rows.append({"position":text["span"]["offset"],"text":text["value"],"speaker":shown,"evidence":proof})
    return rows if settings.get("list_output") else {"rows":rows,"member_filter":steps}
'''
        encrypted=CIPHERS[3,'Full']
        for native_covers_clear,outer_filter,accept,frame,list_output in ((False,False,True,False,False),(True,False,False,False,False),
                (True,True,True,False,False),(False,False,True,True,False),(True,False,False,True,False),
                (False,False,True,False,True),(True,False,False,False,True),(True,False,False,True,True)):
            with self.subTest(native_covers_clear=native_covers_clear,outer_filter=outer_filter,frame=frame,list_output=list_output):
                with tempfile.TemporaryDirectory(prefix='formal-synthetic-xp3-') as folder:
                    root=pathlib.Path(folder)
                    (root/'game.exe').write_bytes(b'Synthetic executable, never run')
                    slot=root/'slot.sav';slot.write_bytes(b'No independently verified current role')
                    archive=root/'native.xp3'
                    payload=mdf(encrypted) if frame else encrypted
                    declared=RAW[3] if native_covers_clear else payload
                    archive.write_bytes(xp3(payload,zlib.adler32(declared)&0xffffffff))
                    agent=HOST['Agent'](dict(game=str(root),save=str(root),skill=str(root),work=str(root/'work'),
                        language='en',title='Synthetic layers',required_saves=[str(slot)]))
                    agent.read_skill=agent.read_contract=agent.used_skill=True
                    extracted=agent.work/'unpacked';extracted.mkdir()
                    member=extracted/'internal.psb';member.write_bytes(payload)
                    agent.extracted_from[str(extracted)]=str(archive)
                    agent.extraction_stamps[str(extracted)]=agent.file_fingerprint(archive)
                    agent.resource_selections[member.name]=dict(selected=str(archive),input_stamps={str(archive):agent.file_fingerprint(archive)})
                    agent.script_resources['synthetic']=dict(key=KEYS[3],outer_filter=outer_filter,frame=frame,list_output=list_output)
                    agent.dispatch('write_program',dict(kind='script',source=program))
                    agent.dispatch('write_program',dict(kind='save',source='def extract(data,metadata): return {"anchors":[],"warnings":["Synthetic unknown role"]}'))
                    args=dict(kind='script',root='work',path=member.relative_to(agent.work).as_posix())
                    with mock.patch.object(agent,'require_script_family'):
                        if accept:
                            result=agent.dispatch('test_program',args)
                            self.assertEqual(result['count'],2)
                            self.assertTrue(result['member_filter_verification']['verified'])
                            if not outer_filter:
                                self.assertEqual(result['member_filter_verification']['stage'],'outer_member_before_document_layers')
                            self.assertTrue(all(record.get('role')=='recorded_node' for row in agent.parts[str(member)] for record in row['_native'] if record['kind']=='typed_record'))
                            self.assertTrue(all(any(record.get('message_reference_only') for record in row['_native']) for row in agent.parts[str(member)]))
                        else:
                            with self.assertRaisesRegex(ValueError,'whole restored member'):
                                agent.dispatch('test_program',args)
                            self.assertEqual(agent.parts,{})
                    with self.assertRaises(ValueError):
                        agent.dispatch('test_program',dict(kind='save',root='save',path=slot.name))
                    self.assertFalse(agent.preparation_status()['ready_for_finish'])
                    with self.assertRaises(ValueError):
                        agent.dispatch('finish',dict(engine='Synthetic',script_language='en',speaker_resolution='inline',notes='No current role was proven'))

    def test_new_layer_archive_authentication_is_required_only_for_new_proof_descriptors(self):
        output=dict(rows=rows_for(CIPHERS[3,'Full'],schema(3)))
        self.assertTrue(ARCHIVES['requires_document_member_verification'](output))
        self.assertTrue(ARCHIVES['requires_document_member_verification'](dict(speakers=dict(a=dict(evidence=output['rows'][0]['evidence'])))))
        self.assertFalse(ARCHIVES['requires_document_member_verification'](dict(rows=rows_for(RAW[3],dict(syntax='psb',view=[])))))
        with tempfile.TemporaryDirectory(prefix='synthetic-auth-') as folder:
            root=pathlib.Path(folder);(root/'game.exe').write_bytes(b'Synthetic')
            agent=HOST['Agent'](dict(game=str(root),save=str(root),skill=str(root),work=str(root/'work'),language='en'))
            member=agent.work/'no-origin.psb';member.write_bytes(CIPHERS[3,'Full'])
            with self.assertRaisesRegex(ValueError,'extraction provenance'):
                agent.verify_member_filter(member,member.read_bytes(),output)
            # Existing no-layer/no-filter trusted output is not newly bound.
            self.assertEqual(agent.verify_member_filter(member,b'trusted-decoded',dict(rows=[])),(b'trusted-decoded',None))
            self.assertEqual(agent.verify_member_filter(member,b'trusted-decoded',[]),(b'trusted-decoded',None))

    def test_formal_bare_psb_refs_cannot_become_current_from_raw_history_or_bmp_integers(self):
        bare_program='''import galgear_structured
def extract(data,metadata):
    document={"syntax":"psb","view":[]}
    root=galgear_structured.parse_document(data,document)["root"]
    rows=[]
    for index in range(2):
        path=["records",index]
        proof={"kind":"typed_field","document":document,"path":path+["text"],"type":"string"}
        selected=galgear_structured.replay_field(data,proof)
        speaker=root["value"]["records"]["value"][index]["value"]["speaker"]
        shown=speaker["value"] if speaker["type"]=="string" else None
        if shown:
            proof["speaker"]={"kind":"typed_field","document":document,"path":path+["speaker"],"type":"string"}
        rows.append({"position":selected["span"]["offset"],"text":selected["value"],"speaker":shown,"evidence":proof})
    return {"rows":rows}
'''
        text=TYPED['replay_field'](RAW[3],field(dict(syntax='psb',view=[]),['records',0,'text']))
        source='synthetic.psb';position=text['span']['offset'];caption=text['value']
        document=dict(syntax='psb',view=[])
        source_proof=field(document,['historical_snapshot','source'])
        anchor_proof=field(document,['historical_snapshot','caption'])
        pointer=TYPED['replay_field'](HISTORICAL_SAVE,field(document,['historical_snapshot','position'],'integer'))
        scalar=dict(offset=pointer['span']['offset']+1,width=pointer['span']['length']-1,endian='little')
        typed_facts=dict(source=source,line=position,anchors=[caption],evidence=dict(source=source_proof,anchor=anchor_proof,position=scalar))
        raw_facts=copy.deepcopy(typed_facts)
        for name,proof in (('source',source_proof),('anchor',anchor_proof)):
            raw_facts['evidence'][name]=TYPED['replay_field'](HISTORICAL_SAVE,proof)['node']['string_span']
        mdf_facts=copy.deepcopy(raw_facts)
        for proof in mdf_facts['evidence'].values():
            proof['view']=[dict(op='mdf')]
        # A valid synthetic thumbnail: its unrelated width happens to equal the
        # script reference. The footer contains only a serialized history tuple.
        pixels=bytes(position*4)
        bitmap=b'BM'+struct.pack('<IHHI',54+len(pixels),0,0,54)+struct.pack('<IiiHHIIiiII',40,position,1,1,32,0,len(pixels),0,0,0,0)+pixels
        footer=('%["history"=>[%["source"=>"'+source+'","caption"=>"'+caption+'"]]]').encode('utf-16-le')
        bmp=bitmap+b'\xff\xfe'+footer
        bmp_facts=dict(source=source,line=position,anchors=[caption],evidence=dict(
            source=dict(offset=bmp.index(source.encode('utf-16-le')),length=len(source.encode('utf-16-le')),encoding='utf-16-le'),
            anchor=dict(offset=bmp.index(caption.encode('utf-16-le')),length=len(caption.encode('utf-16-le')),encoding='utf-16-le'),
            position=dict(offset=18,width=4,endian='little')))
        with tempfile.TemporaryDirectory(prefix='formal-reference-only-') as folder:
            root=pathlib.Path(folder);(root/'game.exe').write_bytes(b'Synthetic executable')
            script=root/source;script.write_bytes(RAW[3]);slot=root/'history.sav'
            agent=HOST['Agent'](dict(game=str(root),save=str(root),skill=str(root),work=str(root/'work'),
                language='en',title='Synthetic reference guard',required_saves=[str(slot)]))
            agent.read_skill=agent.read_contract=agent.used_skill=True
            agent.dispatch('write_program',dict(kind='script',source=bare_program))
            result=agent.dispatch('test_program',dict(kind='script',root='game',path=script.name))
            self.assertEqual(result['count'],2)
            for row in agent.parts[str(script)]:
                self.assertEqual(len(row['_native']),1)
                self.assertEqual(row['_native'][0]['kind'],'psb_message_reference')
                self.assertTrue(row['_native'][0]['message_reference_only'])
                self.assertNotIn('role',row['_native'][0])
            cases=((HISTORICAL_SAVE,typed_facts,'Typed current-node'),(HISTORICAL_SAVE,raw_facts,'PSB current-node'),
                   (mdf(HISTORICAL_SAVE),mdf_facts,'PSB current-node'),(bmp,bmp_facts,'Message-reference'))
            for data,facts,reason in cases:
                with self.subTest(reason=reason):
                    slot.write_bytes(data)
                    agent.dispatch('write_program',dict(kind='save',source='def extract(data,metadata): return '+repr(facts)))
                    with self.assertRaisesRegex((ValueError,RuntimeError),reason):
                        agent.dispatch('test_program',dict(kind='save',root='save',path=slot.name))
                    self.assertFalse(agent.preparation_status()['ready_for_finish'])
                    self.assertEqual(agent.validated_saves(),{})
                    with self.assertRaises(ValueError):
                        agent.dispatch('finish',dict(engine='Synthetic',script_language='en',speaker_resolution='inline',notes='History is not current'))

    def test_formal_psb_speaker_metadata_requires_outer_crc_and_one_complete_typed_root(self):
        typed_program='''import galgear_structured
def extract(data,metadata):
    document={"syntax":"psb","view":[],"frame":{"codec":"mdf"}}
    name={"kind":"typed_field","document":document,"path":["records",0,"speaker"],"type":"string"}
    token={"kind":"typed_field","document":{"syntax":"psb","frame":{"codec":"mdf"}},"path":["records",0,"position"],"type":"integer"}
    shown=galgear_structured.replay_field(data,name)["value"]
    native=galgear_structured.replay_field(data,token)["value"]
    return {"values":{},"speakers":{"one":{"id":native,"text":shown,"evidence":name,"id_evidence":token}}}
'''
        legacy_program='''import galgear_codecs
import galgear_structured
def extract(data,metadata):
    raw=galgear_codecs.decode("mdf",data)
    document={"syntax":"psb","view":[]}
    name=galgear_structured.replay_field(raw,{"kind":"typed_field","document":document,"path":["records",0,"speaker"],"type":"string"})
    token=galgear_structured.replay_field(raw,{"kind":"typed_field","document":document,"path":["records",0,"position"],"type":"integer"})
    return {"view":[{"op":"mdf"}],"values":{},"speakers":{"one":{"id":token["value"],"text":name["value"],"evidence":name["node"]["string_span"],"id_evidence":{"offset":token["span"]["offset"]+1,"width":token["span"]["length"]-1}}}}
'''
        wrapped=mdf(RAW[3])
        evidence_view_program=legacy_program.replace('"view":[{"op":"mdf"}],"values":{}','"values":{}').replace('"evidence":name["node"]["string_span"]','"evidence":dict(name["node"]["string_span"],view=[{"op":"mdf"}])').replace('"width":token["span"]["length"]-1','"width":token["span"]["length"]-1,"view":[{"op":"mdf"}]')
        composite_program=legacy_program.replace('"view":[{"op":"mdf"}]','"view":[{"op":"slice","offset":8,"length":len(data)-8},{"op":"zlib"}]')
        for valid_crc,program,accepted,reason in ((False,typed_program,False,'whole restored member'),
                (False,legacy_program,False,'whole restored member'),(True,legacy_program,False,'complete typed documents'),
                (False,evidence_view_program,False,'whole restored member'),(True,evidence_view_program,False,'complete typed_field'),
                (False,composite_program,False,'whole restored member'),(True,composite_program,False,'complete typed documents'),
                (True,typed_program,True,'')):
            with self.subTest(valid_crc=valid_crc,typed=program==typed_program):
                with tempfile.TemporaryDirectory(prefix='formal-psb-names-') as folder:
                    root=pathlib.Path(folder);(root/'game.exe').write_bytes(b'Synthetic executable')
                    archive=root/'names.xp3';archive.write_bytes(xp3(wrapped,zlib.adler32(wrapped)&0xffffffff if valid_crc else 1))
                    agent=HOST['Agent'](dict(game=str(root),save=str(root),skill=str(root),work=str(root/'work'),language='en'))
                    agent.read_skill=agent.read_contract=agent.used_skill=True
                    output=agent.work/'unpacked';output.mkdir();member=output/'internal.psb';member.write_bytes(wrapped)
                    agent.extracted_from[str(output)]=str(archive)
                    agent.extraction_stamps[str(output)]=agent.file_fingerprint(archive)
                    args=dict(root='work',path=member.relative_to(agent.work).as_posix(),key='names',source=program)
                    if accepted:
                        result=agent.dispatch('load_script_metadata',args)
                        self.assertEqual(result['verified_speakers'],1)
                    else:
                        with self.assertRaisesRegex(ValueError,reason):
                            agent.dispatch('load_script_metadata',args)
                        self.assertNotIn('names',agent.speaker_resources)
                    self.assertFalse(agent.preparation_status()['ready_for_finish'])

    def test_typed_record_never_acquires_current_or_resume_qualification(self):
        encrypted=CIPHERS[3,'Full']
        document=schema(3)
        rows=EVIDENCE['script_rows'](encrypted,dict(rows=rows_for(encrypted,document)))
        native=EVIDENCE['ADAPTERS']['attest_native_nodes'](encrypted,rows,
            span=EVIDENCE['span'],integer=EVIDENCE['integer'],view=EVIDENCE['view'])
        canonical=[dict(source='synthetic.psb',position=row['position'],text=row['text'],speaker=row['speaker'],
                        _native=native[row['position']]) for row in rows]
        self.assertTrue(all(item.get('role')=='recorded_node' for records in native.values() for item in records if item['kind']=='typed_record'))
        self.assertTrue(all(any(item.get('message_reference_only') for item in records) for records in native.values()))
        link=dict(kind='typed_record',document=document,record=['records',0],scope=['scope'],coordinate=['position'],
                  caption=['text'],state_role='recorded_node')
        relation=TYPED['resolve_record'](encrypted,link,canonical)
        self.assertFalse(relation['current_verified'])
        with self.assertRaisesRegex(ValueError,'current/choice/resume role is unverified'):
            EVIDENCE['save_facts'](encrypted,dict(anchors=[rows[0]['text']],evidence=dict(node_link=link)),canonical)
        self.assertFalse(HOST['verified_progress_test'](dict(coverage='recorded_node',facts=relation,matching_anchors=[])))

    def test_original_child_sandbox_uses_fixed_codec_and_contract_versions_agree(self):
        program='''import galgear_structured
def extract(data, metadata):
    proof={"kind":"typed_field","document":{"syntax":"psb","view":[],"crypto":{"codec":"psb_decrypt","key":metadata["key"]}},"path":["records",0,"text"],"type":"string"}
    selected=galgear_structured.replay_field(data,proof)
    return {"text":selected["value"],"offset":selected["span"]["offset"]}
'''
        result=HOST['run_parser'](program,CIPHERS[2,'Full'],dict(key=KEYS[2]))
        expected=TYPED['replay_field'](RAW[2],field(dict(syntax='psb',view=[]),['records',0,'text']))
        self.assertEqual(result,dict(text=expected['value'],offset=expected['span']['offset']))
        self.assertEqual(EVIDENCE['SCRIPT_VERIFICATION_VERSION'],3)
        self.assertEqual(HOST['SCRIPT_VERIFICATION_VERSION'],3)
        self.assertIn('psb_decrypt',CODECS['capabilities']()['codecs'])


if __name__=='__main__':
    unittest.main()
