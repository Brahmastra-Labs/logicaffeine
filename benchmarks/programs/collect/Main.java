import java.util.HashMap;

public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        HashMap<Integer, Integer> map = new HashMap<>(n);
        for (int i = 0; i < n; i++)
            map.put(i, i * 2);
        int found = 0;
        for (int i = 0; i < n; i++)
            if (map.get(i) == i * 2) found++;
        System.out.println(found);
    }
}
